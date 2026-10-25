//! Nginx 配置生成、版本切换与生效确认。
use crate::{
    App,
    error::{Error, Result},
    files,
    model::{Site, SiteKind},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

/// 校验监听冲突与运行目录后生成完整配置版本。
pub fn render(
    app: &App,
    id: &str,
    sites: &[Site],
    performance: &crate::performance::Performance,
) -> Result<String> {
    let performance_directives = performance.directives()?;
    crate::model::validate_domains(sites)?;
    let release = app.instance.join("releases").join(id);
    std::fs::create_dir_all(&release)?;
    let mut listeners: BTreeMap<(std::net::IpAddr, u16), (Option<String>, Vec<String>)> =
        BTreeMap::new();
    for site in sites.iter().filter(|s| s.enabled) {
        site.validate()?;
        let mut addresses = vec![site.listen_ip];
        if site.ipv6_enabled {
            addresses.push(std::net::Ipv6Addr::UNSPECIFIED.into());
        }
        for address in addresses {
            let entry = listeners
                .entry((address, site.port))
                .or_insert((site.certificate_id.clone(), vec![]));
            // 同一 socket 不能混用 HTTP 与 HTTPS。
            if entry.0.is_some() != site.certificate_id.is_some() {
                return Err(Error::bad(
                    "LISTENER_PROTOCOL_CONFLICT",
                    "同一监听端口不能混用 HTTP 和 HTTPS",
                ));
            }
            for domain in &site.domains {
                if entry.1.contains(domain) {
                    return Err(Error::bad(
                        "LISTENER_DOMAIN_CONFLICT",
                        "同一监听端口存在重复域名",
                    ));
                }
                entry.1.push(domain.clone());
            }
        }
        if site.kind == SiteKind::Static
            && !files::resolve(&app.site_root(site), &site.run_dir, false)?.is_dir()
        {
            return Err(Error::with_params(
                "RUN_DIRECTORY_MISSING",
                format!("网站 {} 的运行目录不存在", site.name),
                json!({"name":site.name}),
            ));
        }
    }
    // 拒绝通配 IP 与同端口的具体 IP 混用，避免绑定语义歧义。
    for (ip, port) in listeners.keys() {
        if ip.is_unspecified()
            && listeners
                .keys()
                .any(|(other, p)| p == port && other != ip && other.is_ipv4() == ip.is_ipv4())
        {
            return Err(Error::bad(
                "LISTENER_ADDRESS_CONFLICT",
                "同端口不能混用通配 IP 和具体 IP",
            ));
        }
    }
    let logs = app.instance.join("logs/nginx");
    std::fs::create_dir_all(&logs)?;
    let socket = app.instance.join("logs/engine.sock");
    let mut config = format!(
        "user nginx;\nworker_processes auto;\npid /tmp/veydeck-nginx.pid;\nerror_log {}/error.log warn;\nevents {{ worker_connections 4096; }}\nhttp {{\ninclude /etc/nginx/mime.types;\ndefault_type application/octet-stream;\nserver_tokens off;\nsendfile on;\ndisable_symlinks on;\n{performance_directives}client_body_temp_path /tmp/veydeck-client-body;\nproxy_temp_path /tmp/veydeck-proxy;\nfastcgi_temp_path /tmp/veydeck-fastcgi;\nuwsgi_temp_path /tmp/veydeck-uwsgi;\nscgi_temp_path /tmp/veydeck-scgi;\nmap $http_upgrade $connection_upgrade {{ default upgrade; '' close; }}\nlog_format website escape=json '{{\"requestId\":\"$request_id\",\"time\":\"$time_iso8601\",\"remote\":\"$remote_addr\",\"method\":\"$request_method\",\"uri\":\"$request_uri\",\"status\":$status,\"host\":\"$host\",\"site\":\"$website_id\",\"revision\":\"{}\"}}';\nserver {{ listen unix:{}; set $website_id internal; access_log off; location = /revision {{ default_type text/plain; return 200 '{}'; }} location / {{ return 404; }} }}\n",
        logs.display(),
        id,
        socket.display(),
        id
    );
    let mut ipv6_options = BTreeSet::new();
    for ((ip, port), (cert, _)) in &listeners {
        if sites.iter().any(|s| {
            s.enabled
                && s.port == *port
                && (ip.is_ipv4() || s.ipv6_enabled)
                && s.domains.iter().any(|d| d == "0.0.0.0")
        }) {
            continue;
        }
        let address = std::net::SocketAddr::new(*ip, *port);
        let tls = if cert.is_some() { " ssl" } else { "" };
        let ipv6 = if ip.is_ipv6() {
            ipv6_options.insert(*port);
            " ipv6only=on"
        } else {
            ""
        };
        config.push_str(&format!("server {{ listen {address}{tls}{ipv6} default_server; server_name _; set $website_id default; access_log {}/access.log website;",logs.display()));
        if let Some(cert) = cert {
            config.push_str(&certificate_config(app, cert)?);
        }
        config.push_str("return 444; }\n");
    }
    std::fs::create_dir_all(release.join("conf.d"))?;
    config.push_str(&format!(
        "include {}/conf.d/*.conf;\n}}\n",
        release.display()
    ));
    let global_config = config;
    for site in sites.iter().filter(|s| s.enabled) {
        let mut config = String::new();
        let logs = app.instance.join("logs/sites").join(&site.directory);
        std::fs::create_dir_all(&logs)?;
        let address = std::net::SocketAddr::new(site.listen_ip, site.port);
        let default = if site.domains.iter().any(|d| d == "0.0.0.0") {
            " default_server"
        } else {
            ""
        };
        let names: Vec<_> = site
            .domains
            .iter()
            .filter(|d| *d != "0.0.0.0")
            .cloned()
            .collect();
        let names = if names.is_empty() {
            "_".into()
        } else {
            names.join(" ")
        };
        config.push_str(&format!("server {{\nlisten {address}{}{default};\nserver_name {};\nset $website_id '{}';\naccess_log {}/access.log website;\n",if site.certificate_id.is_some(){" ssl"}else{""},names,site.id,logs.display()));
        if site.ipv6_enabled {
            let tls = if site.certificate_id.is_some() {
                " ssl"
            } else {
                ""
            };
            // 每个 socket 只设置一次 ipv6only，避免多个 vhost 重复声明监听选项。
            let option = if ipv6_options.insert(site.port) {
                " ipv6only=on"
            } else {
                ""
            };
            config.push_str(&format!(
                "listen [::]:{}{tls}{option}{default};\n",
                site.port
            ));
        }
        config.push_str(&format!("error_log {}/error.log warn;\n", logs.display()));
        if let Some(cert) = &site.certificate_id {
            config.push_str(&certificate_config(app, cert)?);
        }
        if site.kind == SiteKind::Static {
            let run = files::resolve(&app.site_root(site), &site.run_dir, false)?;
            config.push_str(&format!("root '{}';\nindex index.html;\nerror_page 404 /404.html;\nlocation = /404.html {{ internal; }}\nlocation / {{ try_files $uri $uri/ {}; }}\n",run.display(),if site.spa{"/index.html"}else{"=404"}));
        } else {
            let url = reqwest::Url::parse(&site.upstream)
                .map_err(|_| Error::bad("PROXY_UPSTREAM_INVALID", "无效上游"))?;
            config.push_str(&format!("location / {{ proxy_pass {}; proxy_http_version 1.1; proxy_set_header Host $host; proxy_set_header X-Real-IP $remote_addr; proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for; proxy_set_header X-Forwarded-Proto $scheme; proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection $connection_upgrade; proxy_connect_timeout 10s; proxy_read_timeout 60s; proxy_ssl_server_name on; proxy_ssl_name {}; proxy_ssl_verify on; proxy_ssl_trusted_certificate /etc/ssl/certs/ca-certificates.crt; }}\n",site.upstream,url.host_str().unwrap_or("")));
        }
        config.push_str(&site.advanced);
        config.push_str("\n}\n");
        files::atomic_write(
            &release
                .join("conf.d")
                .join(format!("{}.conf", site.directory)),
            config.as_bytes(),
        )?;
    }
    let config = global_config;
    files::atomic_write(&release.join("nginx.conf"), config.as_bytes())?;
    // 不把宿主机绝对路径或任意用户字段拼入执行命令。
    Ok(config)
}
/// 解析固定证书版本目录，不允许将用户路径写入 TLS 配置。
fn certificate_config(app: &App, id: &str) -> Result<String> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| Error::bad("CERTIFICATE_DIRECTORY_ID_INVALID", "证书 ID 无效"))?;
    let root = app.instance.join("certificates").join(id);
    if !root.join("certificate.pem").is_file() || !root.join("key.pem").is_file() {
        return Err(Error::bad(
            "CERTIFICATE_REFERENCE_MISSING",
            "引用的证书不存在",
        ));
    }
    Ok(format!(
        "ssl_certificate {}; ssl_certificate_key {}; ssl_protocols TLSv1.2 TLSv1.3;\n",
        root.join("certificate.pem").display(),
        root.join("key.pem").display()
    ))
}
/// 通过临时符号链接原子切换当前配置版本。
pub fn activate(instance: &Path, id: &str) -> Result<()> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| Error::bad("CONFIGURATION_ID_INVALID", "发布 ID 无效"))?;
    let temporary = instance.join(format!(".current-{}", uuid::Uuid::now_v7()));
    std::os::unix::fs::symlink(format!("releases/{id}"), &temporary)?;
    std::fs::rename(temporary, instance.join("current"))?;
    Ok(())
}
/// 读取当前配置指向的版本。
pub fn current(instance: &Path) -> Option<String> {
    std::fs::read_link(instance.join("current"))
        .ok()?
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
}
/// 调用固定 Agent 操作；开发模式不伪造成功。
pub async fn control(app: &App, action: &str, id: Option<&str>) -> Result<serde_json::Value> {
    let runtime = app.runtime.as_ref().ok_or_else(|| {
        Error::bad(
            "ENGINE_RUNTIME_UNAVAILABLE",
            "未连接 Agent Runtime，无法控制引擎",
        )
    })?;
    let response = runtime
        .request(
            reqwest::Method::POST,
            &format!("/api/v1/agent/suite-runtime/services/{action}"),
            Some(&json!({"revision":id})),
        )
        .await
        .map_err(|e| {
            Error::with_params(
                "ENGINE_CONTROL_FAILED",
                e.to_string(),
                json!({"reason":e.to_string()}),
            )
        })?;
    response.json().await.map_err(|e| {
        Error::with_params(
            "ENGINE_CONTROL_FAILED",
            e.to_string(),
            json!({"reason":e.to_string()}),
        )
    })
}
/// 确认正在响应请求的新 worker 已加载目标版本。
pub async fn confirm(app: &App, id: &str) -> Result<()> {
    for _ in 0..40 {
        let client = reqwest::Client::builder()
            .unix_socket(app.instance.join("logs/engine.sock"))
            .no_proxy()
            .timeout(Duration::from_millis(500))
            .build()
            .map_err(|e| {
                Error::with_params(
                    "ENGINE_CONTROL_FAILED",
                    e.to_string(),
                    json!({"reason":e.to_string()}),
                )
            })?;
        if let Ok(response) = client.get("http://localhost/revision").send().await
            && response.text().await.unwrap_or_default() == id
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(Error::bad(
        "ENGINE_CONFIRMATION_TIMEOUT",
        "引擎未确认加载新配置",
    ))
}
