use crate::mods::{
    audit::{AuditContext, AuditEvent, BUSINESS_CODE_HEADER},
    management::AppState,
};
use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web, Error, HttpMessage,
};
use futures::future::LocalBoxFuture;
use qstring::QString;
use std::{
    future::{ready, Ready},
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct AuditTrail;

impl<S, B> Transform<S, ServiceRequest> for AuditTrail
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuditTrailMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuditTrailMiddleware { service }))
    }
}

pub struct AuditTrailMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for AuditTrailMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let path = req.path().to_string();
        let should_audit = !path.starts_with("/admin") && path != "/" && path != "/donate";
        let started = Instant::now();
        let state = req.app_data::<web::Data<AppState>>().cloned();
        let context = if should_audit {
            state.as_ref().map(|state| {
                let query = QString::from(req.query_string());
                let mut event = AuditEvent::new(
                    req.method().as_str(),
                    &path,
                    request_scope(&path),
                    &state.resolve_client_ip(req.request()).to_string(),
                );
                event.area = query.get("area").unwrap_or("").to_string();
                event.ep_id = query.get("ep_id").and_then(|value| value.parse().ok());
                event.season_id = query.get("season_id").and_then(|value| value.parse().ok());
                event.keyword = query.get("keyword").unwrap_or("").to_string();
                Arc::new(Mutex::new(event))
            })
        } else {
            None
        };

        if let Some(context) = context.as_ref() {
            req.extensions_mut().insert::<AuditContext>(context.clone());
        }
        let future = self.service.call(req);

        Box::pin(async move {
            let mut response = future.await?;
            if let (Some(state), Some(context)) = (state, context) {
                if let Ok(mut event) = context.lock() {
                    if let Some(code) = response
                        .headers_mut()
                        .remove(BUSINESS_CODE_HEADER)
                        .next()
                        .and_then(|value| value.to_str().ok()?.parse::<i64>().ok())
                    {
                        event.business_code = Some(code);
                    }
                    event.http_status = response.status().as_u16();
                    event.duration_ms = started.elapsed().as_millis() as u64;
                    state.audit.record(event.clone());
                }
            }
            Ok(response)
        })
    }
}

fn request_scope(path: &str) -> &'static str {
    if path.contains("playurltv") {
        "tv"
    } else if path.contains("playurl") {
        "playurl"
    } else if path.contains("search") {
        "search"
    } else if path.contains("subtitle") {
        "subtitle"
    } else if path.contains("season") {
        "season"
    } else if path.contains("accesskey") {
        "accesskey"
    } else {
        "other"
    }
}
