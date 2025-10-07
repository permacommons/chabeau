pub mod cache;
pub mod card;
pub mod import;
pub mod loader;

#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod tests_integration;

// Public API exports - will be used by other modules in future tasks
#[allow(unused_imports)]
pub use cache::{CachedCardMetadata, CardCache};
#[allow(unused_imports)]
pub use card::{CharacterCard, CharacterData};
#[allow(unused_imports)]
pub use import::{import_card, ImportError};
#[allow(unused_imports)]
pub use loader::{
    find_card_by_name, get_cards_dir, list_available_cards, load_card, load_json_card,
    load_png_card, validate_card, CardLoadError,
};
