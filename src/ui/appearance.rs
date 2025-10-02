/// Preferred appearance used to choose a default theme when none is set
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// Try to detect the preferred appearance via OS-level app theme preference.
/// Returns None if no hint is available.
pub fn detect_preferred_appearance() -> Option<Appearance> {
    detect_via_os_hint()
}

/// Detect OS-level app theme preference (best-effort, with conservative fallbacks)
fn detect_via_os_hint() -> Option<Appearance> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        // `defaults read -g AppleInterfaceStyle` returns "Dark" when dark mode is on.
        if let Ok(output) = Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.to_ascii_lowercase().contains("dark") {
                    return Some(Appearance::Dark);
                } else {
                    return Some(Appearance::Light);
                }
            }
        }
        // If the key is missing, defaults returns a non-zero exit code.
        // Treat that as Light (system default when no Dark value present).
        return Some(Appearance::Light);
    }

    #[cfg(target_os = "windows")]
    {
        // Read HKCU\...\Personalize\AppsUseLightTheme (1 = light, 0 = dark)
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(personalize) =
            hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        {
            let value: Result<u32, _> = personalize.get_value("AppsUseLightTheme");
            if let Ok(v) = value {
                return Some(if v == 0 {
                    Appearance::Dark
                } else {
                    Appearance::Light
                });
            }
        }
        return None; // Unknown
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;
        // GNOME 42+: color-scheme is 'prefer-dark' or 'default'
        if let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let s = stdout.to_ascii_lowercase();
                if s.contains("prefer-dark") {
                    return Some(Appearance::Dark);
                } else if s.contains("default") {
                    return Some(Appearance::Light);
                }
            }
        }
        // Fallback: older GNOME themes often include "-dark" in gtk-theme name
        if let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let s = stdout.to_ascii_lowercase();
                if s.contains("-dark") {
                    return Some(Appearance::Dark);
                } else {
                    return Some(Appearance::Light);
                }
            }
        }
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}
