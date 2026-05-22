use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn format_log_line(source: &str, scenario: Option<&str>, message: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    match scenario {
        Some(s) => format!("[{millis}][{source}][{s}] {message}"),
        None => format!("[{millis}][{source}] {message}"),
    }
}

pub fn log_parent(scenario: &str, message: &str) {
    eprintln!("{}", format_log_line("parent", Some(scenario), message));
}

pub fn spawn_stream_logger<R: Read + Send + 'static>(
    reader: R,
    source: &'static str,
    scenario: String,
) -> (thread::JoinHandle<()>, mpsc::Receiver<String>) {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let formatted = format_log_line(source, Some(&scenario), &line);
                    eprintln!("{formatted}");
                    let _ = tx.send(formatted);
                }
                Err(err) => {
                    let formatted = format_log_line(
                        source,
                        Some(&scenario),
                        &format!("<stream read error: {err}>"),
                    );
                    eprintln!("{formatted}");
                    let _ = tx.send(formatted);
                    break;
                }
            }
        }
    });
    (handle, rx)
}
