// Static-analysis fixtures only; deliberately unsafe examples never compile into the app.
fn bad(client: Client, validation: Validation, input: String) {
    // ruleid: rust-disable-tls-verification
    client.danger_accept_invalid_certs(true);
    // ruleid: rust-disable-tls-verification
    client.danger_accept_invalid_hostnames(true);
    // ruleid: rust-disable-jwt-signature-verification
    validation.insecure_disable_signature_validation();
    // ruleid: rust-disable-jwt-expiry-validation
    validation.validate_exp = false;
    // ruleid: rust-disable-jwt-audience-validation
    validation.validate_aud = false;
    // ruleid: rust-disable-jwt-not-before-validation
    validation.validate_nbf = false;
    // ruleid: rust-permissive-cors
    actix_cors::Cors::permissive();
    // ruleid: rust-permissive-cors
    Cors::permissive();
    // ruleid: rust-permissive-cors
    cors.allow_any_origin();
    // ruleid: rust-permissive-cors
    cors.send_wildcard();
    // ruleid: rust-formatted-sql-query
    sqlx::query(format!("SELECT * FROM users WHERE nama='{}'", input));
    // ruleid: rust-formatted-sql-query
    sqlx::query_scalar(format!("SELECT count(*) FROM users WHERE nama='{}'", input));
    // ruleid: rust-formatted-sql-query
    sqlx::query_as(format!("SELECT * FROM users WHERE nama='{}'", input));
    // ruleid: rust-shell-command-string
    std::process::Command::new("sh").arg("-c").arg(input);
    // ruleid: rust-shell-command-string
    Command::new("bash").arg("-c").arg(input);
    // ruleid: rust-shell-command-string
    tokio::process::Command::new("/bin/bash").arg("-c").arg(input);
}
fn good(client: Client, validation: Validation, input: String) {
    // ok: rust-disable-tls-verification
    client.danger_accept_invalid_certs(false);
    // ok: rust-disable-jwt-expiry-validation
    validation.validate_exp = true;
    // ok: rust-disable-jwt-audience-validation
    validation.validate_aud = true;
    // ok: rust-disable-jwt-not-before-validation
    validation.validate_nbf = true;
    // ok: rust-permissive-cors
    actix_cors::Cors::default().allowed_origin("https://example.test");
    // ok: rust-formatted-sql-query
    sqlx::query("SELECT * FROM users WHERE nama=$1").bind(input);
    // ok: rust-shell-command-string
    std::process::Command::new("pdftotext").arg(input);
}
