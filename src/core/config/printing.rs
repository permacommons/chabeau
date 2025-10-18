use crate::core::config::data::Config;

impl Config {
    pub fn print_all(&self) {
        println!("Current configuration:");
        match &self.default_provider {
            Some(provider) => println!("  default-provider: {provider}"),
            None => println!("  default-provider: (unset)"),
        }
        match &self.theme {
            Some(theme) => println!("  theme: {theme}"),
            None => println!("  theme: (unset)"),
        }
        match self.markdown.unwrap_or(true) {
            true => println!("  markdown: on"),
            false => println!("  markdown: off"),
        }
        match self.syntax.unwrap_or(true) {
            true => println!("  syntax: on"),
            false => println!("  syntax: off"),
        }
        if self.default_models.is_empty() {
            println!("  default-models: (none set)");
        } else {
            println!("  default-models:");
            for (provider, model) in &self.default_models {
                println!("    {provider}: {model}");
            }
        }
        self.print_default_characters();
    }
}
