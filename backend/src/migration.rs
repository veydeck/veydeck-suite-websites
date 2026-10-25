//! 将旧站点目录迁移到创建域名/index，迁移清单允许进程中断后继续。
use crate::{
    App,
    error::{Error, Result},
    files,
    model::{Site, normalize_domain, validate_domains},
};

/// 启动时完成幂等目录迁移，目标存在时绝不覆盖。
pub fn migrate(app: &App) -> Result<()> {
    let mut db = app.db()?;
    let mut sites = db.sites()?;
    if !sites.iter().any(|s| s.directory.is_empty()) {
        return Ok(());
    }
    let journal = app.instance.join("state/directory-migration.json");
    if journal.exists() {
        sites = serde_json::from_slice::<Vec<Site>>(&std::fs::read(&journal)?)?;
    } else {
        for site in &mut sites {
            site.domains = site
                .domains
                .iter()
                .map(|d| normalize_domain(d))
                .collect::<Result<_>>()?;
            site.directory = site
                .domains
                .first()
                .ok_or_else(|| Error::bad("MIGRATION_DOMAIN_REQUIRED", "迁移需要网站主域名"))?
                .clone();
            let target = app.root.join("sites").join(&site.directory);
            if target.exists() {
                return Err(Error::with_params(
                    "MIGRATION_TARGET_EXISTS",
                    format!("迁移目标已存在：{}", target.display()),
                    serde_json::json!({"path":target.display().to_string()}),
                ));
            }
        }
        validate_domains(&sites)?;
        files::atomic_write(&journal, &serde_json::to_vec(&sites)?)?;
    }
    for site in &sites {
        let old = app.root.join("sites").join(&site.id).join("root");
        let target = app.site_root(site);
        if old.exists() {
            if target.exists() {
                return Err(Error::bad(
                    "MIGRATION_DIRECTORY_CONFLICT",
                    "旧目录和目标目录同时存在，停止迁移以保留文件",
                ));
            }
            std::fs::create_dir_all(target.parent().unwrap())?;
            std::fs::rename(&old, &target)?;
            let _ = std::fs::remove_dir(old.parent().unwrap());
        } else if !target.is_dir() {
            return Err(Error::bad("MIGRATION_SOURCE_MISSING", "迁移源目录不存在"));
        }
    }
    let mut releases = db.releases()?;
    for release in &mut releases {
        for site in &mut release.sites {
            if let Some(saved) = sites.iter().find(|s| s.id == site.id) {
                site.directory = saved.directory.clone();
                site.domains = saved.domains.clone();
            }
        }
    }
    let tx = db.0.transaction()?;
    for site in sites {
        tx.execute(
            "UPDATE sites SET document=?2 WHERE id=?1",
            rusqlite::params![site.id, serde_json::to_string(&site)?],
        )?;
    }
    for release in releases {
        tx.execute(
            "UPDATE releases SET document=?2 WHERE id=?1",
            rusqlite::params![release.id, serde_json::to_string(&release)?],
        )?;
    }
    tx.commit()?;
    std::fs::remove_file(journal)?;
    Ok(())
}

/// 清理已退役的运行文件，普通访问和错误日志保持原样。
pub fn cleanup_removed_protection(app: &App) -> Result<()> {
    for name in [
        "state/waf-bans.json",
        "state/waf-runtime-token",
        "logs/waf-manager.sock",
    ] {
        remove_if_present(&app.instance.join(name))?;
    }
    for release in std::fs::read_dir(app.instance.join("releases"))? {
        let release = release?;
        if release.file_type()?.is_dir() {
            remove_if_present(&release.path().join("policy.json"))?;
        }
    }
    let logs = app.instance.join("logs/sites");
    if logs.is_dir() {
        for site in std::fs::read_dir(logs)? {
            let site = site?;
            if !site.file_type()?.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(site.path())? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == "waf.log" || name.starts_with("waf.log.") {
                    remove_if_present(&entry.path())?;
                }
            }
        }
    }
    // 服务日志目录改名，已有日志不丢失；目标已存在时保留旧目录供用户查阅。
    let old = app.instance.join("logs/openresty");
    let target = app.instance.join("logs/nginx");
    if old.is_dir() && !target.exists() {
        std::fs::rename(old, target)?;
    }
    Ok(())
}
/// 幂等删除已退役的单个文件或 Unix socket。
fn remove_if_present(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    /// 模拟旧数据库和文件，验证迁移可重入且不会覆盖上传内容。
    #[test]
    fn migrates_old_files_and_refuses_collision() {
        let root = tempfile::tempdir().unwrap();
        let instance = root.path().join("instance");
        std::fs::create_dir_all(instance.join("state")).unwrap();
        let store = crate::store::Store::open(&instance.join("state/websites.db")).unwrap();
        let id = uuid::Uuid::now_v7().to_string();
        let document = serde_json::json!({"id":id,"revision":1,"name":"旧网站","enabled":true,"domains":["Example.COM."],"listenIp":"127.0.0.1","port":80,"kind":"static","runDir":".","spa":false,"upstream":"","certificateId":null,"advanced":""});
        store
            .0
            .execute(
                "INSERT INTO sites VALUES(?1,1,?2)",
                rusqlite::params![id, document.to_string()],
            )
            .unwrap();
        let app = App {
            root: root.path().into(),
            instance,
            store: Mutex::new(store),
            runtime: None,
            publish_lock: Arc::new(tokio::sync::Mutex::new(())),
            file_lock: Mutex::new(()),
            entry_token: None,
        };
        let old = root.path().join("sites").join(&id).join("root");
        files::initialize(&old).unwrap();
        std::fs::write(old.join("index.html"), "用户上传内容").unwrap();
        let target = root.path().join("sites/example.com");
        std::fs::create_dir_all(&target).unwrap();
        assert!(migrate(&app).is_err());
        assert!(old.exists());
        std::fs::remove_dir(target).unwrap();
        migrate(&app).unwrap();
        migrate(&app).unwrap();
        let site = app.db().unwrap().site(&id).unwrap();
        assert_eq!(site.directory, "example.com");
        assert_eq!(
            std::fs::read_to_string(app.site_root(&site).join("index.html")).unwrap(),
            "用户上传内容"
        );
        assert!(!old.exists());
    }
}
