use axum::{
    body::{Bytes, HttpBody},
    extract::FromRequest,
    http::{header, request, Request, StatusCode},
    response::{IntoResponse, Response},
    BoxError,
};
use prost::Message;
use serde::de::DeserializeOwned;

use crate::prelude::{RpcError, RpcErrorCode};

pub(crate) struct ReqResInto {
    pub binary: bool,
}

pub(crate) fn encode_error(e: &RpcError, for_streaming: bool) -> Vec<u8> {
    if for_streaming {
        // See `encode_message` for the format. It's the same, except always JSON.
        let mut v = vec![0x2, 0, 0, 0, 0];
        serde_json::to_writer(&mut v, &e).unwrap();
        let size = ((v.len() - 5) as u32).to_be_bytes();
        v[1..5].copy_from_slice(&size);

        v
    } else {
        serde_json::to_vec(&e).unwrap()
    }
}

// Encode an error into a Response.
pub(crate) fn encode_error_response(
    e: &RpcError,
    as_binary: bool,
    for_streaming: bool,
) -> Response {
    if for_streaming {
        (
            // Streaming errors ALWAYS return the error in JSON, but the content type still mirrors
            // what ever the request was made with.
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                if as_binary {
                    "application/connect+proto"
                } else {
                    "application/connect+json"
                },
            )],
            encode_error(e, true),
        )
            .into_response()
    } else {
        (
            StatusCode::from(e.code.clone()),
            [(header::CONTENT_TYPE, "application/json")],
            encode_error(e, false),
        )
            .into_response()
    }
}

pub(crate) fn decode_check_headers(
    parts: &mut request::Parts,
    for_streaming: bool,
) -> Result<ReqResInto, Response> {
    // Check the version header, if specified.
    if let Some(version) = parts.headers.get("connect-protocol-version") {
        let version = version.to_str().unwrap_or_default();
        if version != "1" {
            return Err(encode_error_response(
                &RpcError::new(
                    RpcErrorCode::InvalidArgument,
                    format!("Unsupported protocol version: {}", version),
                ),
                true,
                for_streaming,
            ));
        }
    }

    // Decode the content type (binary or JSON).
    // TODO: I'm not sure if this is correct. The Spec doesn't say what content type will be set for
    //       server-streaming responses.
    let binary = match parts.headers.get("content-type") {
        Some(content_type) => match (
            content_type
                .to_str()
                .unwrap_or_default()
                .to_lowercase()
                .split(';')
                .next()
                .unwrap_or_default()
                .trim(),
            for_streaming,
        ) {
            ("application/json", false) => false,
            ("application/proto", false) => true,
            ("application/connect+json", true) => false,
            ("application/connect+proto", true) => true,
            (s, _) => {
                return Err(encode_error_response(
                    &RpcError::new(
                        RpcErrorCode::InvalidArgument,
                        format!("Wrong or unknown Content-Type: {}", s),
                    ),
                    true,
                    true,
                ))
            }
        },
        None => {
            return Err(encode_error_response(
                &RpcError::new(
                    RpcErrorCode::InvalidArgument,
                    "Missing Content-Type header".to_string(),
                ),
                true,
                true,
            ))
        }
    };

    Ok(ReqResInto { binary })
}

pub(crate) async fn decode_request_payload<M, S, B>(
    req: Request<B>,
    state: &S,
    as_binary: bool,
    for_streaming: bool,
) -> Result<M, Response>
where
    M: Message + DeserializeOwned + Default,
    S: Send + Sync + 'static,
    B: Send + Sync + 'static,
    B: HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    // Axum-connect only supports unary request types, so we can ignore for_streaming.
    if as_binary {
        let bytes = match Bytes::from_request(req, state).await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Err(encode_error_response(
                    &RpcError::new(
                        RpcErrorCode::InvalidArgument,
                        format!("Failed to read request body. {}", e),
                    ),
                    as_binary,
                    for_streaming,
                ))
            }
        };

        let message: M = M::decode(bytes).map_err(|e| {
            encode_error_response(
                &RpcError::new(
                    RpcErrorCode::InvalidArgument,
                    format!("Failed to decode binary protobuf. {}", e),
                ),
                as_binary,
                for_streaming,
            )
        })?;

        Ok(message)
    } else {
        let str = match String::from_request(req, state).await {
            Ok(value) => value,
            Err(e) => {
                return Err(encode_error_response(
                    &RpcError::new(
                        RpcErrorCode::InvalidArgument,
                        format!("Failed to read request body. {}", e),
                    ),
                    as_binary,
                    for_streaming,
                ));
            }
        };

        let message: M = serde_json::from_str(&str).map_err(|e| {
            encode_error_response(
                &RpcError::new(
                    RpcErrorCode::InvalidArgument,
                    format!("Failed to decode JSON protobuf. {}", e),
                ),
                as_binary,
                for_streaming,
            )
        })?;

        Ok(message)
    }
}
