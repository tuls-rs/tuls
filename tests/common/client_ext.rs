use rmcp::{
    model::{GetPromptRequestParams, ReadResourceRequestParams, Tool},
    service::ServiceError,
};
use serde_json::Value;

use super::TulsServer;

impl TulsServer {
    pub async fn tools(&self) -> Vec<Tool> {
        self.service.list_all_tools().await.expect("list tools")
    }

    pub async fn get_prompt(&self, name: &str, arguments: Value) -> Result<Value, ServiceError> {
        let request = GetPromptRequestParams::new(name.to_string())
            .with_arguments(arguments.as_object().cloned().unwrap_or_default());
        let response = self.service.get_prompt(request).await?;
        Ok(serde_json::to_value(response).expect("serialize prompt response"))
    }

    pub async fn read_resource(&self, uri: &str) -> Result<Value, ServiceError> {
        let request = ReadResourceRequestParams::new(uri.to_string());
        let response = self.service.read_resource(request).await?;
        Ok(serde_json::to_value(response).expect("serialize resource response"))
    }

    pub async fn list_resources(&self) -> Result<Vec<Value>, ServiceError> {
        let response = self.service.list_resources(None).await?;
        Ok(serde_json::to_value(response)
            .expect("serialize resources response")
            .get("resources")
            .cloned()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default())
    }
}
