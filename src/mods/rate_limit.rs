use super::management::AppState;
use actix_governor::{KeyExtractor, SimpleKeyExtractionError};
use actix_web::{dev::ServiceRequest, http::header::ContentType, web};
// use governor::clock::{Clock, DefaultClock};
use qstring::QString;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct BiliUserToken;

impl KeyExtractor for BiliUserToken {
    type Key = String;
    type KeyExtractionError = SimpleKeyExtractionError<&'static str>;

    fn extract(&self, req: &ServiceRequest) -> Result<Self::Key, Self::KeyExtractionError> {
        let key = QString::from(req.query_string())
            .get("access_key")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                req.app_data::<web::Data<AppState>>()
                    .map(|state| state.resolve_client_ip(req.request()).to_string())
                    .or_else(|| req.peer_addr().map(|address| address.ip().to_string()))
                    .unwrap_or_else(|| "unknown".to_owned())
            });
        Ok(key)
    }

    fn exceed_rate_limit_response(
        &self,
        negative: &actix_governor::governor::NotUntil<
            actix_governor::governor::clock::QuantaInstant,
        >,
        mut response: actix_web::HttpResponseBuilder,
    ) -> actix_web::HttpResponse {
        let wait_time = negative
            .wait_time_from(actix_governor::governor::clock::Clock::now(
                &actix_governor::governor::clock::DefaultClock::default(),
            ))
            .as_secs();
        response.content_type(ContentType::json()).body(format!(
            r#"{{"code":-429,"message":"请求过快,请{wait_time}s后重试"}}"#
        ))
    }
}
