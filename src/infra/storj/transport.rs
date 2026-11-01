//! Bounded HTTP adapter for the official SDK. Provider bodies never enter errors.

use super::{MAX_OBJECT_BYTES, Result, TransportError};
use aws_smithy_runtime_api::client::{
    http::{
        HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
    },
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
    runtime_components::RuntimeComponents,
};
use aws_smithy_types::body::SdkBody;
use reqwest::{Method, StatusCode, blocking::Client};
use std::{io::Read, time::Duration};

#[derive(Clone, Debug)]
pub(super) struct Transport(Client);

impl Transport {
    pub(super) fn new(https_only: bool) -> Result<Self> {
        Client::builder()
            .https_only(https_only)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map(Self)
            .map_err(|_| TransportError::Unavailable)
    }

    fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        let method = Method::from_bytes(request.method().as_bytes())
            .map_err(|_| TransportError::InvalidResponse)?;
        let url = url::Url::parse(request.uri()).map_err(|_| TransportError::InvalidResponse)?;
        let max = if url.query_pairs().any(|(key, _)| key == "list-type") {
            1024 * 1024
        } else {
            MAX_OBJECT_BYTES
        };

        let body = request
            .body()
            .bytes()
            .ok_or(TransportError::InvalidResponse)?;
        let mut outgoing = self.0.request(method, url).body(body.to_vec());

        for (name, value) in request.headers().iter() {
            outgoing = outgoing.header(name, value);
        }

        let response = outgoing.send().map_err(|_| TransportError::Unavailable)?;
        check(response.status())?;

        if response
            .content_length()
            .is_some_and(|length| length > max as u64)
        {
            return Err(TransportError::Budget);
        }

        let mut result = HttpResponse::new(
            response
                .status()
                .as_u16()
                .try_into()
                .map_err(|_| TransportError::InvalidResponse)?,
            SdkBody::empty(),
        );

        for (name, value) in response.headers() {
            result.headers_mut().insert(
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| TransportError::InvalidResponse)?
                    .to_owned(),
            );
        }

        let mut bytes = Vec::new();
        response
            .take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| TransportError::Unavailable)?;

        if bytes.len() > max {
            return Err(TransportError::Budget);
        }
        *result.body_mut() = SdkBody::from(bytes);
        Ok(result)
    }
}

impl HttpClient for Transport {
    fn http_connector(
        &self,
        _: &HttpConnectorSettings,
        _: &RuntimeComponents,
    ) -> SharedHttpConnector {
        SharedHttpConnector::new(self.clone())
    }
}

impl HttpConnector for Transport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let transport = self.clone();
        HttpConnectorFuture::new(async move {
            tokio::task::spawn_blocking(move || transport.send(request))
                .await
                .map_err(|_| ConnectorError::user(Box::new(TransportError::Unavailable)))?
                .map_err(|error| ConnectorError::user(Box::new(error)))
        })
    }
}

pub(super) fn check(status: StatusCode) -> Result<()> {
    match status {
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => Err(TransportError::AccessDenied),
        StatusCode::NOT_FOUND => Err(TransportError::NotFound),
        status if status.is_success() => Ok(()),
        status
            if status.is_server_error()
                || status == StatusCode::TOO_MANY_REQUESTS
                || status == StatusCode::REQUEST_TIMEOUT =>
        {
            Err(TransportError::Unavailable)
        }
        _ => Err(TransportError::InvalidResponse),
    }
}
