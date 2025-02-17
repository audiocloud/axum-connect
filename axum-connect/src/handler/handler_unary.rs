use std::{convert::Infallible, pin::Pin};

use axum::{
    body::HttpBody,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    BoxError,
};
use futures::Future;
use prost::Message;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::RpcIntoError,
    parts::RpcFromRequestParts,
    prelude::{RpcError, RpcErrorCode},
    response::RpcIntoResponse,
};

use super::codec::{
    decode_check_headers, decode_request_payload, encode_error_response, ReqResInto,
};

pub trait RpcHandlerUnary<TMReq, TMRes, TUid, TState, TBody>:
    Clone + Send + Sized + 'static
{
    type Future: Future<Output = Response> + Send + 'static;

    fn call(self, req: Request<TBody>, state: TState) -> Self::Future;
}

// This is for Unary.
// TODO: Check that the header "connect-protocol-version" == "1"
// TODO: Get "connect-timeout-ms" (number as string) and apply timeout.
// TODO: Parse request metadata from:
//      - [0-9a-z]*!"-bin" ASCII value
//      - [0-9a-z]*-bin" (base64 encoded binary)
// TODO: Allow response to send back both leading and trailing metadata.

// This is here because writing Rust macros sucks a**. So I uncomment this when I'm trying to modify
// the below macro.
// #[allow(unused_parens, non_snake_case, unused_mut)]
// impl<TMReq, TMRes, TInto, TFnFut, TFn, TState, TBody, T1>
//     RpcHandlerUnary<TMReq, TMRes, (T1, TMReq), TState, TBody> for TFn
// where
//     TMReq: Message + DeserializeOwned + Default + Send + 'static,
//     TMRes: Message + Serialize + Send + 'static,
//     TInto: RpcIntoResponse<TMRes>,
//     TFnFut: Future<Output = TInto> + Send,
//     TFn: FnOnce(T1, TMReq) -> TFnFut + Clone + Send + 'static,
//     TBody: HttpBody + Send + Sync + 'static,
//     TBody::Data: Send,
//     TBody::Error: Into<BoxError>,
//     TState: Send + Sync + 'static,
//     T1: RpcFromRequestParts<TMRes, TState> + Send,
// {
//     type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

//     fn call(self, req: Request<TBody>, state: TState) -> Self::Future {
//         Box::pin(async move {
//             let (mut parts, body) = req.into_parts();

//             let ReqResInto { binary } = match decode_check_headers(&mut parts, false) {
//                 Ok(binary) => binary,
//                 Err(e) => return e,
//             };

//             let state = &state;

//             let t1 = match T1::rpc_from_request_parts(&mut parts, state).await {
//                 Ok(value) => value,
//                 Err(e) => {
//                     let e = e.rpc_into_error();
//                     return encode_error_response(&e, binary, false);
//                 }
//             };

//             let req = Request::from_parts(parts, body);

//             let proto_req: TMReq = match decode_request_payload(req, state, binary, false).await {
//                 Ok(value) => value,
//                 Err(e) => return e,
//             };

//             let res = self(t1, proto_req).await.rpc_into_response();
//             let res = match res {
//                 Ok(res) => {
//                     if binary {
//                         res.encode_to_vec()
//                     } else {
//                         match serde_json::to_vec(&res) {
//                             Ok(res) => res,
//                             Err(e) => {
//                                 let e = RpcError::new(
//                                     RpcErrorCode::Internal,
//                                     format!("Failed to serialize response: {}", e),
//                                 );
//                                 return encode_error_response(&e, binary, false);
//                             }
//                         }
//                     }
//                 }
//                 Err(e) => {
//                     return encode_error_response(&e, binary, false);
//                 }
//             };

//             (
//                 StatusCode::OK,
//                 [(
//                     header::CONTENT_TYPE,
//                     if binary {
//                         "application/proto"
//                     } else {
//                         "application/json"
//                     },
//                 )],
//                 Result::<Vec<u8>, Infallible>::Ok(res),
//             )
//                 .into_response()
//         })
//     }
// }

macro_rules! impl_handler {
    (
        [$($ty:ident),*]
    ) => {
        #[allow(unused_parens, non_snake_case, unused_mut)]
        impl<TMReq, TMRes, TInto, TFnFut, TFn, TState, TBody, $($ty,)*>
            RpcHandlerUnary<TMReq, TMRes, ($($ty,)* TMReq), TState, TBody> for TFn
        where
            TMReq: Message + DeserializeOwned + Default + Send + 'static,
            TMRes: Message + Serialize + Send + 'static,
            TInto: RpcIntoResponse<TMRes>,
            TFnFut: Future<Output = TInto> + Send,
            TFn: FnOnce($($ty,)* TMReq) -> TFnFut + Clone + Send + 'static,
            TBody: HttpBody + Send + Sync + 'static,
            TBody::Data: Send,
            TBody::Error: Into<BoxError>,
            TState: Send + Sync + 'static,
            $( $ty: RpcFromRequestParts<TMRes, TState> + Send, )*
        {
            type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

            fn call(self, req: Request<TBody>, state: TState) -> Self::Future {
                Box::pin(async move {
                    let (mut parts, body) = req.into_parts();

                    let ReqResInto { binary } = match decode_check_headers(&mut parts, false) {
                        Ok(binary) => binary,
                        Err(e) => return e,
                    };

                    let state = &state;

                    $(
                        let $ty = match $ty::rpc_from_request_parts(&mut parts, state).await {
                            Ok(value) => value,
                            Err(e) => {
                                let e = e.rpc_into_error();
                                return encode_error_response(&e, binary, false);
                            }
                        };
                    )*

                    let req = Request::from_parts(parts, body);

                    let proto_req: TMReq = match decode_request_payload(req, state, binary, false).await {
                        Ok(value) => value,
                        Err(e) => return e,
                    };

                    let res = self($($ty,)* proto_req).await.rpc_into_response();
                    let res = match res {
                        Ok(res) => {
                            if binary {
                                res.encode_to_vec()
                            } else {
                                match serde_json::to_vec(&res) {
                                    Ok(res) => res,
                                    Err(e) => {
                                        let e = RpcError::new(
                                            RpcErrorCode::Internal,
                                            format!("Failed to serialize response: {}", e),
                                        );
                                        return encode_error_response(&e, binary, false);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            return encode_error_response(&e, binary, false);
                        }
                    };

                    (
                        StatusCode::OK,
                        [(
                            header::CONTENT_TYPE,
                            if binary {
                                "application/proto"
                            } else {
                                "application/json"
                            },
                        )],
                        Result::<Vec<u8>, Infallible>::Ok(res),
                    )
                        .into_response()
                })
            }
        }
    };
}

impl_handler!([]);
impl_handler!([T1]);
impl_handler!([T1, T2]);
impl_handler!([T1, T2, T3]);
impl_handler!([T1, T2, T3, T4]);
impl_handler!([T1, T2, T3, T4, T5]);
impl_handler!([T1, T2, T3, T4, T5, T6]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14]);
impl_handler!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15]);
