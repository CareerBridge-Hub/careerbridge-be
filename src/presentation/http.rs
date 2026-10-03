use actix_web::{HttpResponse, Responder, web};

pub fn configure(config: &mut web::ServiceConfig) {
    config.route("/health", web::get().to(health));
}

async fn health() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json")
        .body(r#"{"status":"ok"}"#)
}
