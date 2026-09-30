use std::io::Write;

pub fn log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/music-player-tui.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::log::log(&format!($($arg)*))
    };
}
