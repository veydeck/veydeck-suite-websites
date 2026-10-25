//! 固定站点 root 内的文件管理，拒绝软链接和路径逃逸。
use crate::error::{Error, Result};
use std::path::{Component, Path, PathBuf};

/// 规范化用户提交的相对路径；根目录以点表示。
pub fn relative(value: &str) -> Result<PathBuf> {
    if value.contains(['\0', '\\']) {
        return Err(Error::bad("PATH_CHARACTERS_INVALID", "路径包含无效字符"));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::bad("PATH_OUTSIDE_SITE", "路径必须位于网站 index 内"));
    }
    Ok(path
        .components()
        .filter_map(|c| {
            if let Component::Normal(p) = c {
                Some(p)
            } else {
                None
            }
        })
        .collect())
}
/// 逐级核查目录，不接受任何软链接。
pub fn resolve(root: &Path, value: &str, create_parents: bool) -> Result<PathBuf> {
    let relative = relative(value)?;
    for ancestor in root.ancestors() {
        if std::fs::symlink_metadata(ancestor)?
            .file_type()
            .is_symlink()
        {
            return Err(Error::bad(
                "SITE_DIRECTORY_SYMLINK",
                "网站目录及其父目录不能为软链接",
            ));
        }
    }
    let mut current = root.to_path_buf();
    let parts: Vec<_> = relative.components().collect();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::bad("SYMLINK_ACCESS_DENIED", "不允许访问软链接"));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if index + 1 < parts.len() && create_parents {
                    std::fs::create_dir(&current)?;
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(current)
}
/// 原子写入单个文件，不影响读取旧版本的请求。
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let temporary = path.with_file_name(format!(".upload-{}", uuid::Uuid::now_v7()));
    std::fs::write(&temporary, content)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}
/// 初始化站点固定目录，仅在首次创建时生成默认页面。
pub fn initialize(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    for (name, content) in [
        (
            "index.html",
            "<!doctype html><html lang=\"zh-CN\"><meta charset=\"UTF-8\"><title>网站已就绪</title><h1>网站已就绪</h1><p>请上传您的网站文件。</p></html>",
        ),
        (
            "404.html",
            "<!doctype html><html lang=\"zh-CN\"><meta charset=\"UTF-8\"><title>404</title><h1>404</h1><p>页面不存在。</p></html>",
        ),
    ] {
        if !root.join(name).exists() {
            std::fs::write(root.join(name), content)?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_escape_and_symlink() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["../secret", "/etc/passwd", "a/../../x", "a\\b"] {
            assert!(resolve(dir.path(), name, true).is_err());
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", dir.path().join("escape")).unwrap();
            assert!(resolve(dir.path(), "escape/secret", true).is_err());
        }
        assert_eq!(
            resolve(dir.path(), "dist/index.html", true).unwrap(),
            dir.path().join("dist/index.html")
        );
    }
    #[test]
    fn initialize_preserves_uploaded_page() {
        let dir = tempfile::tempdir().unwrap();
        initialize(dir.path()).unwrap();
        atomic_write(&dir.path().join("index.html"), b"custom").unwrap();
        initialize(dir.path()).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("index.html")).unwrap(),
            b"custom"
        );
    }
}
