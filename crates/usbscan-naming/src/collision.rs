// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

//! 重名处理与原子占位（`docs/04` §7）。

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::NamingError;

/// 重名策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collision {
    /// 追加序号：`名字 (2).jpg`、`名字 (3).jpg`…
    #[default]
    Append,
    /// 直接覆盖（调用方须先二次确认）。
    Overwrite,
    /// 不保存，返回 `None`。
    Skip,
    /// 询问用户。引擎侧无法询问，兜底为追加序号；UI 应在调用前完成询问。
    Ask,
}

/// 已通过原子占位得到的文件。
pub struct ReservedFile {
    /// 完整路径。
    pub path: PathBuf,
    /// 已以独占方式打开、可直接写入的句柄。
    pub file: File,
}

/// 在同一次拍摄中最多尝试多少个候选名。
const MAX_ATTEMPTS: u32 = 10_000;

/// 在 `dir` 下原子占位一个文件名。
///
/// 使用 `CREATE_NEW` 语义（`OpenOptions::create_new`），
/// 不会出现"先检查再创建"的 TOCTOU 竞态。
pub fn reserve(
    dir: &Path,
    stem: &str,
    extension: &str,
    policy: Collision,
) -> Result<Option<ReservedFile>, NamingError> {
    if stem.is_empty() || stem.contains(['\\', '/']) {
        return Err(NamingError::IllegalName);
    }
    fs::create_dir_all(dir)?;

    let extension = extension.trim_start_matches('.');
    match policy {
        Collision::Overwrite => {
            let path = dir.join(join(stem, extension));
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            Ok(Some(ReservedFile { path, file }))
        }
        Collision::Skip => {
            let path = dir.join(join(stem, extension));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => Ok(Some(ReservedFile { path, file })),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
                Err(err) => Err(err.into()),
            }
        }
        Collision::Append | Collision::Ask => {
            for attempt in 1..=MAX_ATTEMPTS {
                let candidate = if attempt == 1 {
                    join(stem, extension)
                } else {
                    join(&format!("{stem} ({attempt})"), extension)
                };
                let path = dir.join(candidate);
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(file) => return Ok(Some(ReservedFile { path, file })),
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(err) => return Err(err.into()),
                }
            }
            Err(NamingError::CollisionExhausted)
        }
    }
}

fn join(stem: &str, extension: &str) -> String {
    if extension.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{extension}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/naming-tests")
            .join(format!(
                "{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn append_policy_adds_suffix() {
        let dir = temp_dir("append");
        let first = reserve(&dir, "扫描_001", "jpg", Collision::Append)
            .unwrap()
            .expect("first");
        let second = reserve(&dir, "扫描_001", "jpg", Collision::Append)
            .unwrap()
            .expect("second");
        assert!(first.path.ends_with("扫描_001.jpg"));
        assert!(second.path.ends_with("扫描_001 (2).jpg"));
        drop((first, second));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skip_policy_returns_none() {
        let dir = temp_dir("skip");
        let first = reserve(&dir, "a", "txt", Collision::Skip).unwrap();
        assert!(first.is_some());
        drop(first);
        assert!(reserve(&dir, "a", "txt", Collision::Skip)
            .unwrap()
            .is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overwrite_policy_reuses_name() {
        let dir = temp_dir("overwrite");
        let mut first = reserve(&dir, "a", "txt", Collision::Append)
            .unwrap()
            .expect("first");
        first.file.write_all(b"one").expect("write");
        drop(first);

        let mut second = reserve(&dir, "a", "txt", Collision::Overwrite)
            .unwrap()
            .expect("overwrite");
        second.file.write_all(b"two").expect("write");
        drop(second);

        let content = fs::read_to_string(dir.join("a.txt")).expect("read");
        assert_eq!(content, "two");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn illegal_stem_rejected() {
        let dir = temp_dir("illegal");
        assert!(matches!(
            reserve(&dir, "a/b", "jpg", Collision::Append),
            Err(NamingError::IllegalName)
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
