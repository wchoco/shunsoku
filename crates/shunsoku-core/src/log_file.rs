//! ログのファイル。大きさが上限を超えそうになったら、今のファイルを `<名前>.old` に 1 世代だけ回す。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 上限で 1 世代だけ回すログのファイル。
pub struct RotatingFile {
    path: PathBuf,
    limit: u64,
    file: File,
    len: u64,
}

impl RotatingFile {
    /// `path` を追記で開く。親のフォルダは作らない。
    pub fn open(path: &Path, limit: u64) -> io::Result<RotatingFile> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let len = file.metadata()?.len();
        Ok(RotatingFile {
            path: path.to_owned(),
            limit,
            file,
            len,
        })
    }

    /// 1 行を書く（改行は付け足す）。
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let mut buf = String::with_capacity(line.len() + 1);
        buf.push_str(line);
        buf.push('\n');
        if self.len > 0 && self.len + buf.len() as u64 > self.limit {
            // 回せなくても（別のプロセスが開いているなど）書き込みは続ける
            let _ = self.rotate();
        }
        self.file.write_all(buf.as_bytes())?;
        self.len += buf.len() as u64;
        Ok(())
    }

    /// 今のファイルを `<path>.old` に置き換えて、空のファイルを開き直す。
    fn rotate(&mut self) -> io::Result<()> {
        let mut old = self.path.clone().into_os_string();
        old.push(".old");
        // std は FILE_SHARE_DELETE で開くので、開いたままでも名前を変えられる
        std::fs::rename(&self.path, &old)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.len = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// テストごとの空のフォルダ。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shunsoku-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn appends_lines_to_existing_file() {
        let dir = temp_dir("append");
        let path = dir.join("shunsoku.log");
        fs::write(&path, "before\n").unwrap();

        let mut log = RotatingFile::open(&path, 1024).unwrap();
        log.write_line("one").unwrap();
        log.write_line("two").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "before\none\ntwo\n");
        assert!(!dir.join("shunsoku.log.old").exists());
    }

    #[test]
    fn moves_current_file_to_old_when_line_would_exceed_limit() {
        let dir = temp_dir("rotate");
        let path = dir.join("shunsoku.log");

        let mut log = RotatingFile::open(&path, 10).unwrap();
        log.write_line("12345").unwrap(); // 6 バイト
        log.write_line("abc").unwrap(); // 10 バイトちょうど。まだ回さない
        log.write_line("x").unwrap(); // 超えるので回す

        assert_eq!(fs::read_to_string(&path).unwrap(), "x\n");
        assert_eq!(
            fs::read_to_string(dir.join("shunsoku.log.old")).unwrap(),
            "12345\nabc\n"
        );
    }

    #[test]
    fn keeps_only_one_old_generation() {
        let dir = temp_dir("one-generation");
        let path = dir.join("shunsoku.log");

        let mut log = RotatingFile::open(&path, 4).unwrap();
        log.write_line("aaa").unwrap();
        log.write_line("bbb").unwrap();
        log.write_line("ccc").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "ccc\n");
        assert_eq!(
            fs::read_to_string(dir.join("shunsoku.log.old")).unwrap(),
            "bbb\n"
        );
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn rotates_existing_oversized_file_on_first_write() {
        let dir = temp_dir("existing");
        let path = dir.join("shunsoku.log");
        fs::write(&path, "0123456789\n").unwrap();

        let mut log = RotatingFile::open(&path, 10).unwrap();
        log.write_line("new").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(
            fs::read_to_string(dir.join("shunsoku.log.old")).unwrap(),
            "0123456789\n"
        );
    }
}
