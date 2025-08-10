# Changelog

## 0.2.0

### Added  

- Added support for Anthropic API provider with proper header handling  
- Configuration management system for default providers and models (`src/core/config.rs`)  
- New CLI commands for setting default model & provider (`src/cli/set_default_model.rs`)  

### Changed  
- Default model selection now automatically uses newest available model when none is configured  

## 0.1.0

Initial release with key features:
- Streaming OpenAI API support
- Scrolling dialog window
- Multi-provider support
- Store keys in system keyring
- External editor support
- Optionally log conversations
- Message retries
