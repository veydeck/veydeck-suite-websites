//! 网站配置及内部恢复的持久化领域模型。
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// 网站配置；保存时校验并重载引擎。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Site {
    pub id: String,
    pub revision: u64,
    #[serde(default)]
    pub directory: String,
    pub name: String,
    pub enabled: bool,
    pub domains: Vec<String>,
    #[serde(default = "default_listen_ip")]
    pub listen_ip: IpAddr,
    #[serde(default)]
    pub ipv6_enabled: bool,
    pub port: u16,
    pub kind: SiteKind,
    pub run_dir: String,
    pub spa: bool,
    pub upstream: String,
    pub certificate_id: Option<String>,
    pub advanced: String,
}
/// 静态网站或 HTTP 反向代理。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SiteKind {
    Static,
    Proxy,
}
impl Site {
    /// 校验用户可编辑配置，防止普通字段注入 Nginx 指令。
    pub fn validate(&self) -> Result<()> {
        require(
            self.revision < i64::MAX as u64,
            "SITE_REVISION_INVALID",
            "网站修订号超出范围",
        )?;
        uuid::Uuid::parse_str(&self.id)
            .map_err(|_| Error::bad("SITE_ID_INVALID", "无效网站 ID"))?;
        require(
            !self.name.trim().is_empty() && self.name.len() <= 128,
            "SITE_NAME_INVALID",
            "网站名称须为 1–128 字节",
        )?;
        require(self.port > 0, "PORT_INVALID", "监听端口不能为 0")?;
        require(
            !self.domains.is_empty() && self.domains.len() <= 64,
            "DOMAIN_COUNT_INVALID",
            "需要 1–64 个域名或 IP",
        )?;
        for domain in &self.domains {
            require(
                normalize_domain(domain)? == *domain,
                "DOMAIN_NOT_NORMALIZED",
                "域名未规范化",
            )?;
        }
        validate_code(&self.directory)?;
        require(
            !self.run_dir.contains(['\n', '\r', '\'', '"', '$']),
            "RUN_DIRECTORY_INVALID",
            "运行目录包含无效字符",
        )?;
        crate::files::relative(&self.run_dir)?;
        if self.kind == SiteKind::Proxy {
            let url = reqwest::Url::parse(&self.upstream)
                .map_err(|_| Error::bad("UPSTREAM_INVALID", "上游地址无效"))?;
            require(
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none(),
                "UPSTREAM_SCHEME_INVALID",
                "上游只支持不含凭据的 HTTP/HTTPS URL",
            )?;
            require(
                !self
                    .upstream
                    .bytes()
                    .any(|b| b.is_ascii_whitespace() || b";{}$\"'\\".contains(&b)),
                "UPSTREAM_CHARACTERS_INVALID",
                "上游地址包含无效字符",
            )?;
            require(
                url.query().is_none() && url.fragment().is_none(),
                "UPSTREAM_QUERY_UNSUPPORTED",
                "上游不能包含查询或片段",
            )?;
        }
        if let Some(id) = &self.certificate_id {
            uuid::Uuid::parse_str(id)
                .map_err(|_| Error::bad("CERTIFICATE_ID_INVALID", "无效证书 ID"))?;
        }
        require(
            self.advanced.len() <= 65536,
            "ADVANCED_CONFIGURATION_TOO_LARGE",
            "高级配置过大",
        )?;
        Ok(())
    }
}
/// 返回可读的业务校验错误。
pub fn require(condition: bool, code: &'static str, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::bad(code, message))
    }
}

/// 规范化域名与 IP，拒绝路径、端口、通配符及空标签。
pub fn normalize_domain(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('.');
    require(
        !value.is_empty() && !value.contains(['/', '\\', ':', '*', '[', ']']),
        "DOMAIN_INPUT_INVALID",
        "请输入域名或 IPv4 地址，不含协议、端口和路径",
    )?;
    let host = reqwest::Url::parse(&format!("http://{value}/"))
        .map_err(|_| Error::bad("DOMAIN_INVALID", "域名格式无效"))?;
    let domain = host
        .host_str()
        .ok_or_else(|| Error::bad("DOMAIN_INVALID", "域名格式无效"))?
        .to_ascii_lowercase();
    require(
        domain.len() <= 253
            && domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }),
        "DOMAIN_INVALID",
        "域名格式无效",
    )?;
    require(
        !value.contains(['@', '?', '#', '%']) && !value.chars().any(char::is_whitespace),
        "DOMAIN_INVALID",
        "域名格式无效",
    )?;
    Ok(domain)
}

/// 默认监听节点的全部 IPv4 网卡。
pub fn default_listen_ip() -> IpAddr {
    std::net::Ipv4Addr::UNSPECIFIED.into()
}

/// 代号用于目录、配置及日志文件名，拒绝路径字符和隐藏目录。
pub fn validate_code(code: &str) -> Result<()> {
    require(
        !code.is_empty()
            && code.len() <= 128
            && code.as_bytes()[0].is_ascii_alphanumeric()
            && code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "SITE_CODE_INVALID",
        "代号须为 1–128 位字母、数字、点、短横线或下划线，以字母或数字开头",
    )
}

/// 代号在节点内唯一，域名仅在同一端口内唯一，停用网站仍保留绑定。
pub fn validate_domains(sites: &[Site]) -> Result<()> {
    let mut domains = std::collections::HashSet::new();
    let mut codes = std::collections::HashSet::new();
    for site in sites {
        validate_code(&site.directory)?;
        if !codes.insert(&site.directory) {
            return Err(Error::with_params(
                "SITE_CODE_EXISTS",
                format!("代号 {} 已被使用，请修改代号", site.directory),
                serde_json::json!({"code":site.directory}),
            ));
        }
        for domain in &site.domains {
            if !domains.insert((site.port, normalize_domain(domain)?)) {
                return Err(Error::with_params(
                    "DOMAIN_ALREADY_BOUND",
                    format!("端口 {} 上的域名 {domain} 已绑定网站", site.port),
                    serde_json::json!({"port":site.port.to_string(),"domain":domain}),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 域名规范化覆盖国际化、大小写和危险路径字符。
    #[test]
    fn canonical_domains() {
        assert_eq!(
            normalize_domain("WWW.Example.COM.").unwrap(),
            "www.example.com"
        );
        assert_eq!(
            normalize_domain("例子.测试").unwrap(),
            "xn--fsqu00a.xn--0zwm56d"
        );
        for value in [
            "",
            "a..b",
            "../bad",
            "user@example.com",
            "x:80",
            "*.example.com",
            "-x.test",
            "example.com?x",
        ] {
            assert!(normalize_domain(value).is_err(), "{value}");
        }
    }
}
