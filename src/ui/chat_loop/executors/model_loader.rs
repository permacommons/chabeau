use crate::api::models::fetch_models;
use crate::core::app::{AppActionContext, AppActionDispatcher, ModelPickerRequest, PickerAction};

pub fn spawn_model_picker_loader(dispatcher: AppActionDispatcher, request: ModelPickerRequest) {
    tokio::spawn(async move {
        let ModelPickerRequest {
            client,
            base_url,
            api_key,
            provider_name,
            default_model_for_provider,
        } = request;

        let fetch_result = fetch_models(&client, &base_url, &api_key, &provider_name)
            .await
            .map_err(|e| e.to_string());

        let action = match fetch_result {
            Ok(models_response) => PickerAction::ModelPickerLoaded {
                default_model_for_provider,
                models_response,
            },
            Err(error) => PickerAction::ModelPickerLoadFailed { error },
        };

        dispatcher.dispatch_many([action], AppActionContext::default());
    });
}
