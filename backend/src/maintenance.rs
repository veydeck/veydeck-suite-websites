//! 一致性备份与有界日志保留。
use crate::{
    App, engine,
    error::{Error, Result},
};
use axum::{
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::Response,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 持有发布与文件锁生成包含一致性 SQLite 快照的下载归档。
pub async fn backup(State(app): State<Arc<App>>) -> Result<Response> {
    let _publish = app
        .publish_lock
        .clone()
        .try_lock_owned()
        .map_err(|_| Error::bad("BACKUP_CONFIGURATION_BUSY", "发布过程中不能备份"))?;
    let output = std::env::temp_dir().join(format!(
        "veydeck-websites-backup-{}.tar.gz",
        uuid::Uuid::now_v7()
    ));
    let path = output.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let _files = app
            .file_lock
            .lock()
            .map_err(|_| Error::bad("FILES_BUSY", "文件管理忙"))?;
        let snapshot = path.with_extension("sqlite");
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&snapshot)?;
        app.db()?.0.backup("main", &snapshot, None)?;
        let result = (|| -> Result<()> {
            let output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            let compressor = flate2::write::GzEncoder::new(output, flate2::Compression::default());
            let mut archive = tar::Builder::new(compressor);
            archive.follow_symlinks(false);
            append(
                &mut archive,
                &app.root,
                &app.root,
                &app.instance.join("state/websites.db"),
            )?;
            archive.append_path_with_name(
                &snapshot,
                app.instance
                    .join("state/websites.db")
                    .strip_prefix(&app.root)
                    .map_err(|_| Error::bad("BACKUP_PATH_INVALID", "备份路径无效"))?,
            )?;
            archive.into_inner()?.finish()?;
            Ok(())
        })();
        let _ = std::fs::remove_file(snapshot);
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    })
    .await
    .map_err(|_| Error::bad("BACKUP_INTERRUPTED", "备份任务中断"))??;
    let file = tokio::fs::File::open(&output).await?;
    tokio::fs::remove_file(output).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=websites-backup.tar.gz",
        )
        .body(Body::from_stream(tokio_util::io::ReaderStream::new(file)))
        .map_err(|_| Error::bad("BACKUP_RESPONSE_FAILED", "无法创建备份响应"))
}
/// 归档业务目录，排除活动数据库和运行 socket。
fn append<W: std::io::Write>(
    archive: &mut tar::Builder<W>,
    root: &Path,
    path: &Path,
    database: &Path,
) -> Result<()> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path == database
            || path == database.with_file_name("websites.db-wal")
            || path == database.with_file_name("websites.db-shm")
            || path.extension().is_some_and(|s| s == "sock")
        {
            continue;
        }
        let name = path
            .strip_prefix(root)
            .map_err(|_| Error::bad("BACKUP_PATH_INVALID", "备份路径无效"))?;
        if entry.file_type()?.is_dir() {
            archive.append_dir(name, &path)?;
            append(archive, root, &path, database)?;
        } else {
            archive.append_path_with_name(&path, name)?;
        }
    }
    Ok(())
}
/// 每日轮转引擎文件日志，保留七天；不更改网站配置。
pub async fn rotate_logs(app: Arc<App>) {
    let mut day = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400;
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 86400;
        if now == day {
            continue;
        }
        let _guard = app.publish_lock.lock().await;
        let mut directories = vec![app.instance.join("logs/nginx")];
        if let Ok(entries) = std::fs::read_dir(app.instance.join("logs/sites")) {
            directories.extend(
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path()),
            );
        }
        let mut renamed = vec![];
        for logs in &directories {
            for name in ["access.log", "error.log"] {
                let from = logs.join(name);
                let to = logs.join(format!("{name}.{day}"));
                if tokio::fs::rename(&from, &to).await.is_ok() {
                    renamed.push((from, to));
                }
            }
        }
        if !renamed.is_empty()
            && let Err(error) = engine::control(&app, "reopen", None).await
        {
            tracing::error!(%error,"日志重新打开失败，恢复文件名");
            for (from, to) in renamed {
                let _ = tokio::fs::rename(to, from).await;
            }
            continue;
        }
        for logs in &directories {
            if let Ok(mut entries) = tokio::fs::read_dir(logs).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if let Some(old) = name.rsplit('.').next().and_then(|s| s.parse::<u64>().ok())
                        && old + 7 < now
                    {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
        day = now;
    }
}
