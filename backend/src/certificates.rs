//! 从已上传的 X.509 证书解析身份、颁发者、有效期及技术信息。
use crate::{
    App,
    error::{Error, Result},
    files,
};
use openssl::{
    asn1::{Asn1Time, Asn1TimeRef},
    hash::MessageDigest,
    nid::Nid,
    pkey::Id,
    x509::{X509, X509NameRef},
};
use serde::{Deserialize, Serialize};

/// 证书公开元数据；时间为 UTC Unix 毫秒，不包含私钥或自定义名称。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Certificate {
    pub id: String,
    pub domain: String,
    pub domains: Vec<String>,
    pub issuer_organization: String,
    pub subject: String,
    pub issuer: String,
    pub serial_number: String,
    pub fingerprint_sha256: String,
    pub signature_algorithm: String,
    pub public_key: String,
    pub not_before: i64,
    pub not_after: i64,
    pub chain_length: usize,
}
/// 解析证书链首张证书，SAN 优先作为网站身份，兼容只有 CN 的证书。
pub fn parse(id: &str, pem: &[u8]) -> Result<Certificate> {
    let chain = X509::stack_from_pem(pem)
        .map_err(|_| Error::bad("CERTIFICATE_CHAIN_INVALID", "证书链 PEM 无效"))?;
    let leaf = chain
        .first()
        .ok_or_else(|| Error::bad("CERTIFICATE_CHAIN_EMPTY", "证书链不能为空"))?;
    let names = |name: &X509NameRef, nid| -> Result<String> {
        name.entries_by_nid(nid)
            .map(|e| {
                e.data().to_string().map_err(|_| {
                    Error::bad("CERTIFICATE_NAME_ENCODING_INVALID", "证书名称编码无效")
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(|values| values.join(" / "))
    };
    let cn = names(leaf.subject_name(), Nid::COMMONNAME)?;
    let mut domains = vec![];
    if let Some(alternatives) = leaf.subject_alt_names() {
        for name in alternatives {
            let value = if let Some(dns) = name.dnsname() {
                Some(dns.to_string())
            } else if let Some(ip) = name.ipaddress() {
                match ip.len() {
                    4 => {
                        Some(std::net::Ipv4Addr::from(<[u8; 4]>::try_from(ip).unwrap()).to_string())
                    }
                    16 => Some(
                        std::net::Ipv6Addr::from(<[u8; 16]>::try_from(ip).unwrap()).to_string(),
                    ),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(value) = value
                && !domains.contains(&value)
            {
                domains.push(value);
            }
        }
    } else if !cn.is_empty() {
        domains.push(cn.clone());
    }
    let domain = domains
        .iter()
        .find(|d| **d == cn)
        .or(domains.first())
        .cloned()
        .unwrap_or_default();
    let organization = names(leaf.issuer_name(), Nid::ORGANIZATIONNAME)?;
    let issuer_organization = if organization.is_empty() {
        names(leaf.issuer_name(), Nid::COMMONNAME)?
    } else {
        organization
    };
    let key = leaf
        .public_key()
        .map_err(|_| Error::bad("CERTIFICATE_PUBLIC_KEY_INVALID", "证书公钥无效"))?;
    let kind = match key.id() {
        Id::RSA => "RSA",
        Id::EC => "EC",
        Id::ED25519 => "Ed25519",
        Id::ED448 => "Ed448",
        Id::DSA => "DSA",
        _ => "其他",
    };
    let fingerprint = leaf
        .digest(MessageDigest::sha256())
        .map_err(|_| Error::bad("CERTIFICATE_FINGERPRINT_INVALID", "证书指纹解析失败"))?;
    Ok(Certificate {
        id: id.into(),
        domain,
        domains,
        issuer_organization,
        subject: distinguished_name(leaf.subject_name())?,
        issuer: distinguished_name(leaf.issuer_name())?,
        serial_number: leaf
            .serial_number()
            .to_bn()
            .and_then(|n| n.to_hex_str())
            .map_err(|_| Error::bad("CERTIFICATE_SERIAL_INVALID", "证书序列号解析失败"))?
            .to_string(),
        fingerprint_sha256: fingerprint
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
        signature_algorithm: leaf.signature_algorithm().object().to_string(),
        public_key: format!("{kind} {} bit", key.bits()),
        not_before: timestamp(leaf.not_before())?,
        not_after: timestamp(leaf.not_after())?,
        chain_length: chain.len(),
    })
}
/// 将 ASN.1 时间统一为浏览器可直接构造日期的 UTC 毫秒。
fn timestamp(value: &Asn1TimeRef) -> Result<i64> {
    let epoch = Asn1Time::from_unix(0)
        .map_err(|_| Error::bad("CERTIFICATE_TIME_INVALID", "证书时间解析失败"))?;
    let diff = epoch
        .diff(value)
        .map_err(|_| Error::bad("CERTIFICATE_TIME_INVALID", "证书时间解析失败"))?;
    Ok((i64::from(diff.days) * 86400 + i64::from(diff.secs)) * 1000)
}
/// 展示完整主题与颁发者，不丢失组织等多值字段。
fn distinguished_name(value: &X509NameRef) -> Result<String> {
    value
        .entries()
        .map(|e| {
            Ok(format!(
                "{}={}",
                e.object(),
                e.data().to_string().map_err(|_| Error::bad(
                    "CERTIFICATE_NAME_ENCODING_INVALID",
                    "证书名称编码无效"
                ))?
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|v| v.join(", "))
}
/// 升级时从证书文件重建元数据，保留证书 ID、文件与网站引用。
pub fn refresh_metadata(app: &App) -> Result<()> {
    for entry in std::fs::read_dir(app.instance.join("certificates"))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !entry.path().join("metadata.json").is_file() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().into_owned();
        uuid::Uuid::parse_str(&id)
            .map_err(|_| Error::bad("CERTIFICATE_STORAGE_ID_INVALID", "证书目录标识无效"))?;
        let certificate = parse(&id, &std::fs::read(entry.path().join("certificate.pem"))?)?;
        files::atomic_write(
            &entry.path().join("metadata.json"),
            &serde_json::to_vec(&certificate)?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::{
        bn::BigNum,
        ec::{EcGroup, EcKey},
        pkey::PKey,
        x509::{X509NameBuilder, extension::SubjectAlternativeName},
    };
    /// 构造带中文颁发组织、多个 SAN 和固定有效期的测试证书。
    fn sample(with_san: bool) -> Vec<u8> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(Nid::COMMONNAME, "legacy.example")
            .unwrap();
        name.append_entry_by_nid(Nid::ORGANIZATIONNAME, "测试证书组织")
            .unwrap();
        let name = name.build();
        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        builder
            .set_serial_number(&BigNum::from_u32(42).unwrap().to_asn1_integer().unwrap())
            .unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(&Asn1Time::from_unix(1700000000).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::from_unix(1800000000).unwrap())
            .unwrap();
        if with_san {
            let extension = SubjectAlternativeName::new()
                .dns("*.example.com")
                .dns("other.example.com")
                .ip("192.0.2.1")
                .ip("2001:db8::1")
                .build(&builder.x509v3_context(None, None))
                .unwrap();
            builder.append_extension(extension).unwrap();
        }
        builder.sign(&key, MessageDigest::sha256()).unwrap();
        builder.build().to_pem().unwrap()
    }
    /// SAN 不包含 CN 时不将旧 CN 误认成网站身份；UTC 时间与指纹保持准确。
    #[test]
    fn extracts_identity_and_technical_fields() {
        let pem = sample(true);
        let cert = parse("test", &[pem.clone(), pem.clone()].concat()).unwrap();
        assert_eq!(cert.domain, "*.example.com");
        assert_eq!(
            cert.domains,
            [
                "*.example.com",
                "other.example.com",
                "192.0.2.1",
                "2001:db8::1"
            ]
        );
        assert_eq!(cert.issuer_organization, "测试证书组织");
        assert!(cert.subject.contains("legacy.example"));
        assert_eq!(cert.serial_number, "2A");
        assert_eq!(cert.public_key, "EC 256 bit");
        assert_eq!(cert.chain_length, 2);
        assert_eq!(cert.not_before, 1700000000000);
        assert_eq!(cert.not_after, 1800000000000);
        assert_eq!(cert.fingerprint_sha256.split(':').count(), 32);
        assert!(!cert.signature_algorithm.is_empty());
        let legacy = parse("test", &sample(false)).unwrap();
        assert_eq!(legacy.domain, "legacy.example");
        assert!(parse("test", b"not a certificate").is_err());
        assert!(parse("test", b"").is_err());
    }
}
