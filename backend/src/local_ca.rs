//! 实例私有根 CA 与本地网站证书签发，仅下载根证书公钥。
use crate::{
    App, api,
    certificates::{self, Certificate},
    error::{Error, Result},
    files,
    model::require,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::{get, post},
};
use openssl::{
    asn1::Asn1Time,
    bn::{BigNum, MsbOption},
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{
        X509, X509NameBuilder,
        extension::{
            AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
            SubjectAlternativeName, SubjectKeyIdentifier,
        },
    },
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path as FsPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

/// 稳定的实例内默认标识；每个实例实际生成不同的密钥和证书。
const DEFAULT_ID: &str = "00000000-0000-0000-0000-000000000001";
/// 根证书公开信息，包含现有证书详情字段。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authority {
    #[serde(flatten)]
    pub certificate: Certificate,
    pub name: String,
    pub organization: String,
    pub is_default: bool,
    pub created_at: i64,
}
/// 只允许明确支持的密钥算法。
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    #[default]
    Ec256,
    Rsa2048,
    Rsa4096,
}
/// 新根 CA 的身份及生命周期。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAuthority {
    pub name: String,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub algorithm: Algorithm,
    #[serde(default = "default_years")]
    pub years: u32,
}
/// 服务器证书必须明确列出 SAN，签发过程不接收任何扩展或命令。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueCertificate {
    pub domains: Vec<String>,
    #[serde(default)]
    pub algorithm: Algorithm,
    #[serde(default = "default_years")]
    pub years: u32,
}
/// 注册根证书列表、创建、签发与仅公钥下载接口。
pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/api/certificate-authorities", get(list).post(create))
        .route("/api/certificate-authorities/{id}/issue", post(issue))
        .route("/api/certificate-authorities/{id}/download", get(download))
}
/// 获取当前时间，拒绝在无效系统时钟下生成信任材料。
fn now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::bad("SYSTEM_TIME_INVALID", "系统时间无效"))?
        .as_secs() as i64)
}
/// 转换加密库错误，不返回任何私钥材料。
fn crypto(error: openssl::error::ErrorStack) -> Error {
    Error::with_params(
        "CERTIFICATE_CRYPTO_FAILED",
        format!("证书操作失败：{error}"),
        serde_json::json!({"reason":error.to_string()}),
    )
}
/// 创建受限目录，根 CA 的文件不会暴露给普通网站 worker。
fn private_directory(path: &FsPath) -> Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
/// 首次启动生成默认根 CA；旧版默认根复用原密钥迁移到标准未定义到期日。
pub fn initialize(app: &App) -> Result<()> {
    initialize_directory(&app.instance.join("authorities"))
}
/// 在实例独立目录完成根证书初始化，避免不同实例共享密钥。
fn initialize_directory(root: &FsPath) -> Result<()> {
    private_directory(root)?;
    let path = root.join(DEFAULT_ID);
    if path.exists() {
        let cert = X509::from_pem(&std::fs::read(path.join("certificate.pem"))?).map_err(crypto)?;
        let key =
            PKey::private_key_from_pem(&std::fs::read(path.join("key.pem"))?).map_err(crypto)?;
        require(
            cert.public_key().map_err(crypto)?.public_eq(&key),
            "DEFAULT_AUTHORITY_KEY_MISMATCH",
            "默认根证书和私钥不匹配，请从备份恢复",
        )?;
        let mut metadata: Authority =
            serde_json::from_slice(&std::fs::read(path.join("metadata.json"))?)?;
        if metadata.name == "Veydeck 本地根证书" {
            metadata.name = "Veydeck".into();
        }
        let end = forever()?;
        if cert.not_after() != end {
            let pem = root_certificate(
                cert.subject_name(),
                &key,
                cert.serial_number(),
                cert.not_before(),
                &end,
            )?;
            files::atomic_write(&path.join("certificate.pem"), &pem)?;
        }
        // 从最终 PEM 重建元数据，覆盖两文件切换中断后可能残留的旧到期时间。
        metadata.certificate =
            certificates::parse(DEFAULT_ID, &std::fs::read(path.join("certificate.pem"))?)?;
        files::atomic_write(&path.join("metadata.json"), &serde_json::to_vec(&metadata)?)?;
        std::fs::set_permissions(path.join("key.pem"), std::fs::Permissions::from_mode(0o600))?;
    } else {
        create_root(
            root,
            DEFAULT_ID,
            CreateAuthority {
                name: "Veydeck".into(),
                organization: "Veydeck".into(),
                algorithm: Algorithm::Ec256,
                years: 10,
            },
        )?;
    }
    Ok(())
}
/// 所有外部标识先验证 UUID，再拼接实例固定目录。
fn authority_path(app: &App, id: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(id).map_err(|_| Error::bad("AUTHORITY_ID_INVALID", "根证书 ID 无效"))?;
    let path = app.instance.join("authorities").join(id);
    require(
        path.join("metadata.json").is_file(),
        "AUTHORITY_NOT_FOUND",
        "根证书不存在",
    )?;
    Ok(path)
}
/// 列表仅返回公开元数据，默认根排在最前。
async fn list(State(app): State<Arc<App>>) -> Result<Json<Vec<Authority>>> {
    let mut values = vec![];
    for entry in std::fs::read_dir(app.instance.join("authorities"))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
        {
            continue;
        }
        values.push(serde_json::from_slice::<Authority>(&std::fs::read(
            entry.path().join("metadata.json"),
        )?)?);
    }
    values.sort_by_key(|a| (!a.is_default, a.created_at, a.certificate.id.clone()));
    Ok(Json(values))
}
/// 密钥生成离开异步执行线程，发布锁同时保护备份一致性。
async fn create(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(input): Json<CreateAuthority>,
) -> Result<Json<Authority>> {
    tokio::spawn(async move {
        let _lock = app.publish_lock.lock().await;
        let root = app.instance.join("authorities");
        let value = tokio::task::spawn_blocking(move || {
            create_root(&root, &uuid::Uuid::now_v7().to_string(), input)
        })
        .await
        .map_err(|_| Error::bad("AUTHORITY_GENERATE_INTERRUPTED", "根证书生成任务中断"))??;
        api::audit(
            app.clone(),
            "certificate_authority_create",
            "创建本地根证书",
            &headers,
        )
        .await;
        Ok(Json(value))
    })
    .await
    .map_err(|_| Error::bad("AUTHORITY_CREATE_INTERRUPTED", "根证书创建任务中断"))?
}
/// 签发结果直接进入网站证书目录，接口不返回私钥。
async fn issue(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<IssueCertificate>,
) -> Result<Json<Certificate>> {
    tokio::spawn(async move {
        let _lock = app.publish_lock.lock().await;
        let path = authority_path(&app, &id)?;
        let root = app.instance.join("certificates");
        let value = tokio::task::spawn_blocking(move || issue_certificate(&path, &root, input))
            .await
            .map_err(|_| Error::bad("CERTIFICATE_ISSUE_INTERRUPTED", "证书签发任务中断"))??;
        api::audit(app.clone(), "certificate_issue", "签发网站证书", &headers).await;
        Ok(Json(value))
    })
    .await
    .map_err(|_| Error::bad("CERTIFICATE_ISSUE_INTERRUPTED", "证书签发任务中断"))?
}
/// 下载内容固定为根证书 PEM；不接受文件名或私钥选项。
async fn download(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    let path = authority_path(&app, &id)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-pem-file".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=veydeck-root-{id}.pem"),
            ),
        ],
        std::fs::read(path.join("certificate.pem"))?,
    ))
}
/// 按白名单生成私钥，不允许任意曲线和低强度 RSA。
fn key(algorithm: &Algorithm) -> Result<PKey<Private>> {
    match algorithm {
        Algorithm::Ec256 => {
            let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).map_err(crypto)?;
            PKey::from_ec_key(EcKey::generate(&group).map_err(crypto)?).map_err(crypto)
        }
        Algorithm::Rsa2048 => PKey::from_rsa(Rsa::generate(2048).map_err(crypto)?).map_err(crypto),
        Algorithm::Rsa4096 => PKey::from_rsa(Rsa::generate(4096).map_err(crypto)?).map_err(crypto),
    }
}
/// 产生随机正序列号，避免跨请求或重启时重复使用计数。
fn serial() -> Result<openssl::asn1::Asn1Integer> {
    let mut value = BigNum::new().map_err(crypto)?;
    value.rand(159, MsbOption::ONE, false).map_err(crypto)?;
    value.to_asn1_integer().map_err(crypto)
}
/// 在私有临时目录内写完整材料，再原子提交；私钥自创建起即为 0600。
fn persist<T: Serialize>(
    root: &FsPath,
    id: &str,
    pem: &[u8],
    key: &PKey<Private>,
    metadata: &T,
) -> Result<()> {
    let temporary = root.join(format!(".pending-{}", uuid::Uuid::now_v7()));
    private_directory(&temporary)?;
    let result = (|| -> Result<()> {
        files::atomic_write(&temporary.join("certificate.pem"), pem)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(temporary.join("key.pem"))?;
        file.write_all(&key.private_key_to_pem_pkcs8().map_err(crypto)?)?;
        file.sync_all()?;
        files::atomic_write(
            &temporary.join("metadata.json"),
            &serde_json::to_vec(metadata)?,
        )?;
        std::fs::rename(&temporary, root.join(id))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(temporary);
    }
    result
}
/// 创建仅用于签发服务器证书的自签根，禁止下级 CA。
fn create_root(root: &FsPath, id: &str, input: CreateAuthority) -> Result<Authority> {
    let name = input.name.trim();
    let organization = input.organization.trim();
    require(
        !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control),
        "AUTHORITY_NAME_INVALID",
        "根证书名称须为 1–64 字节，不含控制字符",
    )?;
    require(
        organization.len() <= 128 && !organization.chars().any(char::is_control),
        "ORGANIZATION_INVALID",
        "组织须为 0–128 字节，不含控制字符",
    )?;
    require(
        (1..=99).contains(&input.years),
        "AUTHORITY_VALIDITY_INVALID",
        "根证书有效期须为 1–99 年",
    )?;
    let key = key(&input.algorithm)?;
    let mut subject = X509NameBuilder::new().map_err(crypto)?;
    subject
        .append_entry_by_nid(Nid::COMMONNAME, name)
        .map_err(crypto)?;
    if !organization.is_empty() {
        subject
            .append_entry_by_nid(Nid::ORGANIZATIONNAME, organization)
            .map_err(crypto)?;
    }
    let subject = subject.build();
    let seconds = now()?;
    let start = Asn1Time::from_unix(seconds - 300).map_err(crypto)?;
    let end = if id == DEFAULT_ID {
        forever()?
    } else {
        calendar_end(seconds, input.years)?
    };
    let serial = serial()?;
    let pem = root_certificate(&subject, &key, &serial, &start, &end)?;
    let value = Authority {
        certificate: certificates::parse(id, &pem)?,
        name: name.into(),
        organization: organization.into(),
        is_default: id == DEFAULT_ID,
        created_at: seconds * 1000,
    };
    persist(root, id, &pem, &key, &value)?;
    Ok(value)
}
/// 网站证书缺省签发十个日历年。
fn default_years() -> u32 {
    10
}
/// 默认根采用标准约定的未定义到期日，不使用无法编码的无限值。
fn forever() -> Result<Asn1Time> {
    Asn1Time::from_str_x509("99991231235959Z").map_err(crypto)
}
/// 按 UTC 日历年计算有效期，闰日跨到平年时取该月最后一天。
fn calendar_end(seconds: i64, years: u32) -> Result<Asn1Time> {
    let start = chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .ok_or_else(|| Error::bad("CERTIFICATE_TIME_OUT_OF_RANGE", "证书时间超出范围"))?;
    let end = start
        .checked_add_months(chrono::Months::new(years * 12))
        .ok_or_else(|| Error::bad("CERTIFICATE_VALIDITY_OUT_OF_RANGE", "证书有效期超出范围"))?;
    Asn1Time::from_unix(end.timestamp()).map_err(crypto)
}
/// 保持既有主体、公钥与序列号，构建根证书标准扩展并自签。
fn root_certificate(
    subject: &openssl::x509::X509NameRef,
    key: &PKey<Private>,
    serial: &openssl::asn1::Asn1IntegerRef,
    start: &openssl::asn1::Asn1TimeRef,
    end: &openssl::asn1::Asn1TimeRef,
) -> Result<Vec<u8>> {
    let mut builder = X509::builder().map_err(crypto)?;
    builder.set_version(2).map_err(crypto)?;
    builder.set_serial_number(serial).map_err(crypto)?;
    builder.set_subject_name(subject).map_err(crypto)?;
    builder.set_issuer_name(subject).map_err(crypto)?;
    builder.set_pubkey(key).map_err(crypto)?;
    builder.set_not_before(start).map_err(crypto)?;
    builder.set_not_after(end).map_err(crypto)?;
    builder
        .append_extension(
            BasicConstraints::new()
                .critical()
                .ca()
                .pathlen(0)
                .build()
                .map_err(crypto)?,
        )
        .map_err(crypto)?;
    builder
        .append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .crl_sign()
                .build()
                .map_err(crypto)?,
        )
        .map_err(crypto)?;
    let ski = SubjectKeyIdentifier::new()
        .build(&builder.x509v3_context(None, None))
        .map_err(crypto)?;
    builder.append_extension(ski).map_err(crypto)?;
    builder.sign(key, MessageDigest::sha256()).map_err(crypto)?;
    builder.build().to_pem().map_err(crypto)
}
/// 校验 DNS、通配域名与 IP，去重后生成明确的 SAN 身份。
fn normalize_domains(values: Vec<String>) -> Result<Vec<String>> {
    require(
        !values.is_empty() && values.len() <= 64,
        "DOMAIN_COUNT_INVALID",
        "需要 1–64 个域名或 IP",
    )?;
    let mut result = vec![];
    for value in values {
        require(
            value.len() <= 512,
            "DOMAIN_TOO_LONG",
            "域名或 IP 长度超出限制",
        )?;
        let value = value.trim();
        let value = if let Ok(ip) = value.parse::<std::net::IpAddr>() {
            ip.to_string()
        } else if let Some(domain) = value.strip_prefix("*.") {
            require(
                domain.parse::<std::net::IpAddr>().is_err(),
                "IP_WILDCARD_UNSUPPORTED",
                "IP 地址不能使用通配符",
            )?;
            format!("*.{}", crate::model::normalize_domain(domain)?)
        } else {
            crate::model::normalize_domain(value)?
        };
        if !result.contains(&value) {
            result.push(value);
        }
    }
    Ok(result)
}
/// 根 CA 签发带 serverAuth 和 SAN 的叶证书，结束时间不超过根有效期。
fn issue_certificate(
    authority: &FsPath,
    root: &FsPath,
    input: IssueCertificate,
) -> Result<Certificate> {
    require(
        (1..=99).contains(&input.years),
        "CERTIFICATE_VALIDITY_INVALID",
        "网站证书有效期须为 1–99 年",
    )?;
    let domains = normalize_domains(input.domains)?;
    let ca = X509::from_pem(&std::fs::read(authority.join("certificate.pem"))?).map_err(crypto)?;
    let ca_key =
        PKey::private_key_from_pem(&std::fs::read(authority.join("key.pem"))?).map_err(crypto)?;
    require(
        ca.public_key().map_err(crypto)?.public_eq(&ca_key),
        "AUTHORITY_KEY_MISMATCH",
        "根证书与私钥不匹配",
    )?;
    let seconds = now()?;
    let current = Asn1Time::from_unix(seconds).map_err(crypto)?;
    require(
        ca.not_before() <= current && ca.not_after() > current,
        "AUTHORITY_NOT_VALID",
        "根证书尚未生效或已经过期",
    )?;
    let key = key(&input.algorithm)?;
    let mut name = X509NameBuilder::new().map_err(crypto)?;
    name.append_entry_by_nid(
        Nid::COMMONNAME,
        if domains[0].len() <= 64 {
            &domains[0]
        } else {
            "Veydeck Website"
        },
    )
    .map_err(crypto)?;
    let mut builder = X509::builder().map_err(crypto)?;
    builder.set_version(2).map_err(crypto)?;
    let serial = serial()?;
    builder.set_serial_number(&serial).map_err(crypto)?;
    builder.set_subject_name(&name.build()).map_err(crypto)?;
    builder.set_issuer_name(ca.subject_name()).map_err(crypto)?;
    builder.set_pubkey(&key).map_err(crypto)?;
    let start = Asn1Time::from_unix(seconds - 300).map_err(crypto)?;
    builder
        .set_not_before(if ca.not_before() > start {
            ca.not_before()
        } else {
            &start
        })
        .map_err(crypto)?;
    let end = calendar_end(seconds, input.years)?;
    builder
        .set_not_after(if ca.not_after() < end {
            ca.not_after()
        } else {
            &end
        })
        .map_err(crypto)?;
    builder
        .append_extension(BasicConstraints::new().critical().build().map_err(crypto)?)
        .map_err(crypto)?;
    let mut usage = KeyUsage::new();
    usage.critical().digital_signature();
    if matches!(input.algorithm, Algorithm::Rsa2048 | Algorithm::Rsa4096) {
        usage.key_encipherment();
    }
    builder
        .append_extension(usage.build().map_err(crypto)?)
        .map_err(crypto)?;
    builder
        .append_extension(
            ExtendedKeyUsage::new()
                .server_auth()
                .build()
                .map_err(crypto)?,
        )
        .map_err(crypto)?;
    let mut names = SubjectAlternativeName::new();
    for domain in &domains {
        if domain.parse::<std::net::IpAddr>().is_ok() {
            names.ip(domain);
        } else {
            names.dns(domain);
        }
    }
    let context = builder.x509v3_context(Some(&ca), None);
    let san = names.build(&context).map_err(crypto)?;
    let ski = SubjectKeyIdentifier::new()
        .build(&context)
        .map_err(crypto)?;
    let aki = AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&context)
        .map_err(crypto)?;
    builder.append_extension(san).map_err(crypto)?;
    builder.append_extension(ski).map_err(crypto)?;
    builder.append_extension(aki).map_err(crypto)?;
    builder
        .sign(&ca_key, MessageDigest::sha256())
        .map_err(crypto)?;
    let mut pem = builder.build().to_pem().map_err(crypto)?;
    pem.extend(ca.to_pem().map_err(crypto)?);
    let id = uuid::Uuid::now_v7().to_string();
    let certificate = certificates::parse(&id, &pem)?;
    persist(root, &id, &pem, &key, &certificate)?;
    Ok(certificate)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 默认根重启复用，独立实例不同密钥，损坏时不得静默重建。
    #[test]
    fn default_root_persists_and_is_instance_private() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        initialize_directory(first.path()).unwrap();
        let cert = std::fs::read(first.path().join(DEFAULT_ID).join("certificate.pem")).unwrap();
        let path = first.path().join(DEFAULT_ID).join("key.pem");
        let key = std::fs::read(&path).unwrap();
        let metadata_path = first.path().join(DEFAULT_ID).join("metadata.json");
        let mut metadata: Authority =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.name, "Veydeck");
        metadata.name = "Veydeck 本地根证书".into();
        std::fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        initialize_directory(first.path()).unwrap();
        assert_eq!(key, std::fs::read(&path).unwrap());
        assert_eq!(
            cert,
            std::fs::read(first.path().join(DEFAULT_ID).join("certificate.pem")).unwrap()
        );
        let metadata: Authority =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.name, "Veydeck");
        initialize_directory(second.path()).unwrap();
        assert_ne!(
            cert,
            std::fs::read(second.path().join(DEFAULT_ID).join("certificate.pem")).unwrap()
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&path, b"corrupted key").unwrap();
        assert!(initialize_directory(first.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupted key");
    }
    /// 自签根能验证叶证书签名，SAN 完整且结束时间被根证书限制。
    #[test]
    fn issuance_is_server_only_and_capped_at_root_expiry() {
        let roots = tempfile::tempdir().unwrap();
        let certificates = tempfile::tempdir().unwrap();
        let ca = create_root(
            roots.path(),
            "00000000-0000-0000-0000-000000000002",
            CreateAuthority {
                name: "测试 CA".into(),
                organization: "测试组织".into(),
                algorithm: Algorithm::Ec256,
                years: 1,
            },
        )
        .unwrap();
        let path = roots.path().join("00000000-0000-0000-0000-000000000002");
        let leaf = issue_certificate(
            &path,
            certificates.path(),
            IssueCertificate {
                domains: vec![
                    "LOCALHOST".into(),
                    "*.Example.com".into(),
                    "192.0.2.1".into(),
                    "2001:db8::1".into(),
                    "localhost".into(),
                ],
                algorithm: Algorithm::Rsa2048,
                years: 10,
            },
        )
        .unwrap();
        assert_eq!(
            leaf.domains,
            ["localhost", "*.example.com", "192.0.2.1", "2001:db8::1"]
        );
        assert_eq!(leaf.not_after, ca.certificate.not_after);
        assert_eq!(leaf.chain_length, 2);
        assert_eq!(leaf.public_key, "RSA 2048 bit");
        let root = X509::from_pem(&std::fs::read(path.join("certificate.pem")).unwrap()).unwrap();
        let issued = X509::from_pem(
            &std::fs::read(certificates.path().join(&leaf.id).join("certificate.pem")).unwrap(),
        )
        .unwrap();
        let key = root.public_key().unwrap();
        assert!(issued.verify(&key).unwrap());
        assert!(root.verify(&key).unwrap());
        let text = String::from_utf8(issued.to_text().unwrap()).unwrap();
        assert!(text.contains("CA:FALSE"));
        assert!(text.contains("TLS Web Server Authentication"));
        let root_text = String::from_utf8(root.to_text().unwrap()).unwrap();
        assert!(root_text.contains("CA:TRUE, pathlen:0"));
        for domains in [
            vec![],
            vec!["http://example.com".into()],
            vec!["*.192.0.2.1".into()],
            vec!["x.test\nDNS:evil.test".into()],
        ] {
            assert!(normalize_domains(domains).is_err());
        }
    }
    /// 跨闰年的年数按日历计算，不用固定 365 天替代一年。
    #[test]
    fn validity_uses_calendar_years() {
        let leap = chrono::DateTime::parse_from_rfc3339("2024-02-29T12:34:56Z")
            .unwrap()
            .timestamp();
        let next = calendar_end(leap, 1).unwrap();
        let expected = Asn1Time::from_str_x509("20250228123456Z").unwrap();
        assert!(next == expected);
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-25T12:34:56Z")
            .unwrap()
            .timestamp();
        assert!(
            calendar_end(start, 10).unwrap() == Asn1Time::from_str_x509("20361025123456Z").unwrap()
        );
    }
    /// 旧默认根升级保持密钥、序列号、主体和创建时间，并且可重复执行。
    #[test]
    fn default_root_expiry_migration_preserves_identity() {
        let root = tempfile::tempdir().unwrap();
        let issued = tempfile::tempdir().unwrap();
        initialize_directory(root.path()).unwrap();
        let path = root.path().join(DEFAULT_ID);
        let key_bytes = std::fs::read(path.join("key.pem")).unwrap();
        let key = PKey::private_key_from_pem(&key_bytes).unwrap();
        let cert = X509::from_pem(&std::fs::read(path.join("certificate.pem")).unwrap()).unwrap();
        let end = calendar_end(now().unwrap(), 10).unwrap();
        let old = root_certificate(
            cert.subject_name(),
            &key,
            cert.serial_number(),
            cert.not_before(),
            &end,
        )
        .unwrap();
        let metadata: Authority =
            serde_json::from_slice(&std::fs::read(path.join("metadata.json")).unwrap()).unwrap();
        std::fs::write(path.join("certificate.pem"), &old).unwrap();
        let leaf = issue_certificate(
            &path,
            issued.path(),
            IssueCertificate {
                domains: vec!["localhost".into()],
                algorithm: Algorithm::Ec256,
                years: 1,
            },
        )
        .unwrap();
        initialize_directory(root.path()).unwrap();
        assert_eq!(std::fs::read(path.join("key.pem")).unwrap(), key_bytes);
        let new_bytes = std::fs::read(path.join("certificate.pem")).unwrap();
        let new = X509::from_pem(&new_bytes).unwrap();
        assert!(new.not_after() == forever().unwrap());
        assert_eq!(
            cert.subject_name().to_der().unwrap(),
            new.subject_name().to_der().unwrap()
        );
        assert_eq!(
            cert.serial_number().to_bn().unwrap(),
            new.serial_number().to_bn().unwrap()
        );
        let updated: Authority =
            serde_json::from_slice(&std::fs::read(path.join("metadata.json")).unwrap()).unwrap();
        assert_eq!(updated.created_at, metadata.created_at);
        assert_eq!(updated.certificate.id, DEFAULT_ID);
        assert_eq!(updated.certificate.not_after, 253402300799000);
        let server = X509::from_pem(
            &std::fs::read(issued.path().join(leaf.id).join("certificate.pem")).unwrap(),
        )
        .unwrap();
        let public = new.public_key().unwrap();
        assert!(server.verify(&public).unwrap());
        initialize_directory(root.path()).unwrap();
        assert_eq!(
            std::fs::read(path.join("certificate.pem")).unwrap(),
            new_bytes
        );
    }
}
