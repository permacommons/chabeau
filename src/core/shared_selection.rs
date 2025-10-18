use crate::core::config::data::Config;
use std::collections::HashMap;

pub(crate) trait ManagedItem: Clone {
    fn id(&self) -> &str;
}

pub(crate) struct SelectionState<T: ManagedItem> {
    items: Vec<T>,
    active: Option<T>,
    defaults: HashMap<(String, String), String>,
    set_default_fn: fn(&mut Config, String, String, String),
    unset_default_fn: fn(&mut Config, &str, &str),
    item_label: &'static str,
}

impl<T: ManagedItem> SelectionState<T> {
    pub(crate) fn load_from_config(
        config: &Config,
        items_getter: impl Fn(&Config) -> &Vec<T>,
        defaults_getter: impl Fn(&Config) -> &HashMap<String, HashMap<String, String>>,
        set_default_fn: fn(&mut Config, String, String, String),
        unset_default_fn: fn(&mut Config, &str, &str),
        item_label: &'static str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut defaults = HashMap::new();
        for (provider, models) in defaults_getter(config) {
            let provider_key = provider.to_lowercase();
            for (model, item_id) in models {
                defaults.insert((provider_key.clone(), model.clone()), item_id.clone());
            }
        }

        Ok(Self {
            items: items_getter(config).clone(),
            active: None,
            defaults,
            set_default_fn,
            unset_default_fn,
            item_label,
        })
    }

    pub(crate) fn items(&self) -> &Vec<T> {
        &self.items
    }

    pub(crate) fn find_by_id(&self, id: &str) -> Option<&T> {
        self.items.iter().find(|item| item.id() == id)
    }

    pub(crate) fn set_active(&mut self, item_id: &str) -> Result<(), String> {
        if let Some(item) = self.items.iter().find(|item| item.id() == item_id).cloned() {
            self.active = Some(item);
            Ok(())
        } else {
            let available_ids: Vec<&str> = self.items.iter().map(|item| item.id()).collect();
            Err(format!(
                "{} '{}' not found. Available {}s: {}",
                self.item_label,
                item_id,
                self.item_label.to_lowercase(),
                available_ids.join(", ")
            ))
        }
    }

    pub(crate) fn clear_active(&mut self) {
        self.active = None;
    }

    pub(crate) fn get_active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    pub(crate) fn get_default_for_provider_model(
        &self,
        provider: &str,
        model: &str,
    ) -> Option<&str> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.get(&key).map(|s| s.as_str())
    }

    pub(crate) fn set_default_persistent(
        &mut self,
        provider: &str,
        model: &str,
        item_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.insert(key, item_id.to_string());

        let provider = provider.to_string();
        let model = model.to_string();
        let item_id = item_id.to_string();
        let setter = self.set_default_fn;

        Config::mutate(move |config| {
            setter(config, provider.clone(), model.clone(), item_id.clone());
            Ok(())
        })?;

        Ok(())
    }

    pub(crate) fn unset_default_persistent(
        &mut self,
        provider: &str,
        model: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let key = (provider.to_lowercase(), model.to_string());
        self.defaults.remove(&key);

        let provider = provider.to_string();
        let model = model.to_string();
        let unsetter = self.unset_default_fn;

        Config::mutate(move |config| {
            unsetter(config, &provider, &model);
            Ok(())
        })?;

        Ok(())
    }
}
