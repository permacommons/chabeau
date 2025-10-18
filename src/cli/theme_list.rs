use crate::core::config::data::Config;
use crate::ui::builtin_themes::load_builtin_themes;

pub async fn list_themes() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let current_id_for_mark = config.theme.as_deref().unwrap_or("dark");
    let current_display = config.theme.as_deref().unwrap_or("(default: dark)");

    println!("Available themes:\n");
    println!("Built-in:");
    for t in load_builtin_themes() {
        let mark = if t.id.eq_ignore_ascii_case(current_id_for_mark) {
            "*"
        } else {
            " "
        };
        println!("  {} {} - {}", mark, t.id, t.display_name);
    }

    let customs = config.list_custom_themes();
    if !customs.is_empty() {
        println!("\nCustom:");
        for t in customs {
            let mark = if t.id.eq_ignore_ascii_case(current_id_for_mark) {
                "*"
            } else {
                " "
            };
            println!("  {} {} - {}", mark, t.id, t.display_name);
        }
    }

    println!("\nCurrent: {}", current_display);
    Ok(())
}
