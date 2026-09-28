//! stdout/stderr capture: bounded in-memory ring plus a log file
//! (SPEC §12 stdout/stderr capture, §29 `tpt logs`).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tpt_runtime_core::error::Result;

/// A thread-safe bounded line buffer.
#[derive(Debug)]
pub struct LogBuffer {
    lines: Mutex<VecDeque<String>>,
    capacity: usize,
}

impl LogBuffer {
    /// Creates a buffer holding the most recent `capacity` lines.
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
            capacity,
        }
    }

    /// Appends a line, dropping the oldest when full.
    pub fn push(&self, line: impl Into<String>) {
        let mut lines = self.lines.lock().unwrap();
        if lines.len() == self.capacity {
            lines.pop_front();
        }
        lines.push_back(line.into());
    }

    /// Returns up to `count` most recent lines, oldest first.
    pub fn tail(&self, count: usize) -> Vec<String> {
        let lines = self.lines.lock().unwrap();
        let start = lines.len().saturating_sub(count);
        lines.iter().skip(start).cloned().collect()
    }

    /// Total lines currently buffered.
    pub fn len(&self) -> usize {
        self.lines.lock().unwrap().len()
    }

    /// Whether nothing is buffered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One captured stream (stdout or stderr) mirrored to memory and a file.
#[derive(Debug)]
pub struct LogCapture {
    /// Bounded memory ring for quick `tpt logs` reads.
    pub buffer: std::sync::Arc<LogBuffer>,
    /// Full log file path.
    pub file: PathBuf,
}

impl LogCapture {
    /// Creates a capture writing to `dir/stdout.log` or `dir/stderr.log`.
    pub fn create(dir: &Path, stream: &str) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file = dir.join(format!("{stream}.log"));
        Ok(Self {
            buffer: std::sync::Arc::new(LogBuffer::new(2048)),
            file,
        })
    }

    /// Spawns a background thread that copies `reader` lines into the
    /// buffer and appends them to the log file until EOF.
    pub fn spawn_reader(&self, reader: impl std::io::Read + Send + 'static) {
        let buffer = self.buffer.clone();
        let path = self.file.clone();
        std::thread::spawn(move || {
            let mut file = File::options()
                .create(true)
                .append(true)
                .open(&path)
                .ok();
            let reader = BufReader::new(reader);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        buffer.push(line.clone());
                        if let Some(file) = file.as_mut() {
                            let _ = writeln!(file, "{line}");
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_keeps_most_recent_lines() {
        let buffer = LogBuffer::new(3);
        for i in 0..5 {
            buffer.push(format!("line-{i}"));
        }
        assert_eq!(buffer.tail(10), vec!["line-2", "line-3", "line-4"]);
        assert_eq!(buffer.tail(2), vec!["line-3", "line-4"]);
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn capture_writes_file_and_buffer() {
        let dir = std::env::temp_dir().join(format!(
            "tpt-logs-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let capture = LogCapture::create(&dir, "stdout").unwrap();
        let reader = std::io::Cursor::new(b"hello\nworld\n".to_vec());
        capture.spawn_reader(reader);

        // wait briefly for the reader thread
        for _ in 0..50 {
            if capture.buffer.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(capture.buffer.tail(2), vec!["hello", "world"]);
        let contents = std::fs::read_to_string(capture.file).unwrap();
        assert_eq!(contents, "hello\nworld\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
