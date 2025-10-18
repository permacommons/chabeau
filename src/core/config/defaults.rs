use crate::core::config::data::Config;

impl Config {
    pub fn get_default_model(&self, provider: &str) -> Option<&String> {
        let normalized = provider.to_lowercase();
        self.default_models
            .get(&normalized)
            .or_else(|| self.default_models.get(provider))
    }

    pub fn set_default_model(&mut self, provider: String, model: String) {
        let normalized = provider.to_lowercase();
        self.default_models.insert(normalized.clone(), model);
        if normalized != provider {
            self.default_models.remove(&provider);
        }
    }

    pub fn unset_default_model(&mut self, provider: &str) {
        let normalized = provider.to_lowercase();
        self.default_models.remove(&normalized);
        if normalized != provider {
            self.default_models.remove(provider);
        }
    }

    pub fn get_default_character(&self, provider: &str, model: &str) -> Option<&String> {
        self.default_characters
            .get(&provider.to_lowercase())
            .and_then(|models| models.get(model))
    }

    pub fn set_default_character(
        &mut self,
        provider: String,
        model: String,
        character_name: String,
    ) {
        let provider_key = provider.to_lowercase();
        self.default_characters
            .entry(provider_key)
            .or_default()
            .insert(model, character_name);
    }

    pub fn unset_default_character(&mut self, provider: &str, model: &str) {
        if let Some(models) = self.default_characters.get_mut(&provider.to_lowercase()) {
            models.remove(model);
            if models.is_empty() {
                self.default_characters.remove(&provider.to_lowercase());
            }
        }
    }

    pub fn set_default_persona(&mut self, provider: String, model: String, persona_id: String) {
        let provider_key = provider.to_lowercase();
        self.default_personas
            .entry(provider_key)
            .or_default()
            .insert(model, persona_id);
    }

    pub fn unset_default_persona(&mut self, provider: &str, model: &str) {
        if let Some(models) = self.default_personas.get_mut(&provider.to_lowercase()) {
            models.remove(model);
            if models.is_empty() {
                self.default_personas.remove(&provider.to_lowercase());
            }
        }
    }

    pub fn set_default_preset(&mut self, provider: String, model: String, preset_id: String) {
        let provider_key = provider.to_lowercase();
        self.default_presets
            .entry(provider_key)
            .or_default()
            .insert(model, preset_id);
    }

    pub fn unset_default_preset(&mut self, provider: &str, model: &str) {
        if let Some(models) = self.default_presets.get_mut(&provider.to_lowercase()) {
            models.remove(model);
            if models.is_empty() {
                self.default_presets.remove(&provider.to_lowercase());
            }
        }
    }

    pub fn print_default_characters(&self) {
        if self.default_characters.is_empty() {
            println!("  default-characters: (none set)");
        } else {
            println!("  default-characters:");
            let mut providers: Vec<_> = self.default_characters.iter().collect();
            providers.sort_by_key(|(k, _)| *k);
            for (provider, models) in providers {
                let mut model_entries: Vec<_> = models.iter().collect();
                model_entries.sort_by_key(|(k, _)| *k);
                for (model, character) in model_entries {
                    println!("    {}:{}: {}", provider, model, character);
                }
            }
        }
    }
}
