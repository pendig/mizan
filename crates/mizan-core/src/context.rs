use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub request_id: Uuid,
    pub trace_id: Uuid,
    pub user_id: Option<Uuid>,
    pub api_key_id: Option<Uuid>,
    pub provider: Option<String>,
    pub provider_id: Option<Uuid>,
    pub route: Option<String>,
    pub route_id: Option<Uuid>,
    pub model: Option<String>,
    pub streaming: bool,
}

impl RequestContext {
    pub fn new() -> Self {
        let request_id = Uuid::now_v7();

        Self {
            request_id,
            trace_id: request_id,
            user_id: None,
            api_key_id: None,
            provider: None,
            provider_id: None,
            route: None,
            route_id: None,
            model: None,
            streaming: false,
        }
    }

    pub fn builder() -> RequestContextBuilder {
        RequestContextBuilder::default()
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct RequestContextBuilder {
    context: RequestContext,
}

impl RequestContextBuilder {
    pub fn request_id(mut self, request_id: Uuid) -> Self {
        self.context.request_id = request_id;
        self
    }

    pub fn trace_id(mut self, trace_id: Uuid) -> Self {
        self.context.trace_id = trace_id;
        self
    }

    pub fn user_id(mut self, user_id: Uuid) -> Self {
        self.context.user_id = Some(user_id);
        self
    }

    pub fn api_key_id(mut self, api_key_id: Uuid) -> Self {
        self.context.api_key_id = Some(api_key_id);
        self
    }

    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.context.provider = Some(provider.into());
        self
    }

    pub fn provider_id(mut self, provider_id: Uuid) -> Self {
        self.context.provider_id = Some(provider_id);
        self
    }

    pub fn route(mut self, route: impl Into<String>) -> Self {
        self.context.route = Some(route.into());
        self
    }

    pub fn route_id(mut self, route_id: Uuid) -> Self {
        self.context.route_id = Some(route_id);
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.context.model = Some(model.into());
        self
    }

    pub fn streaming(mut self, streaming: bool) -> Self {
        self.context.streaming = streaming;
        self
    }

    pub fn build(self) -> RequestContext {
        self.context
    }
}
