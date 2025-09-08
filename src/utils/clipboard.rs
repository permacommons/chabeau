use std::io::Write;
use std::process::{Command, Stdio};

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return run_with_stdin("pbcopy", &[], text);
    }
    #[cfg(target_os = "windows")]
    {
        return run_with_stdin("cmd", &["/C", "clip"], text);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if run_with_stdin("wl-copy", &[], text).is_ok() {
            return Ok(());
        }
        if run_with_stdin("xclip", &["-selection", "clipboard"], text).is_ok() {
            return Ok(());
        }
        if run_with_stdin("xsel", &["--clipboard", "--input"], text).is_ok() {
            return Ok(());
        }
        Err("No clipboard command found (install wl-copy, xclip, or xsel)".to_string())
    }
}

fn run_with_stdin(cmd: &str, args: &[&str], input: &str) -> Result<(), String> {
    match Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(input.as_bytes());
            }
            match child.wait() {
                Ok(status) if status.success() => Ok(()),
                _ => Err(format!("Clipboard command `{}` failed", cmd)),
            }
        }
        Err(_) => Err(format!("Clipboard command `{}` not available", cmd)),
    }
}
