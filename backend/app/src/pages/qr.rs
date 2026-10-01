//! `/qr?data=<url>`: a QR code for the device flow's
//! `verification_uri_complete`, so someone can approve a pairing from their
//! phone. Only links to this server: the page can't be used to make QR
//! codes for other sites.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum_extra::extract::Query;
use base64::Engine;
use serde::Deserialize;

use super::security::{message, render, AppOrigin};
use super::views::QrPage;
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct QrInput {
    #[serde(default)]
    data: String,
}

/// `GET /qr`
pub(crate) async fn show(State(state): State<AppState>, Query(input): Query<QrInput>) -> Response {
    let origin = AppOrigin::of(&state);
    if !origin.owns(&input.data) {
        return message(
            &origin,
            StatusCode::BAD_REQUEST,
            "Not a link to this server",
            "This page only makes QR codes for links to this server.",
            None,
        );
    }
    let svg = match qrcode::QrCode::new(input.data.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(240, 240)
            .build(),
        Err(_) => {
            return message(
                &origin,
                StatusCode::BAD_REQUEST,
                "That link is too long",
                "It doesn't fit in a QR code.",
                None,
            )
        }
    };
    let image = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg)
    );
    render(
        StatusCode::OK,
        &QrPage {
            title: "Scan to connect".into(),
            host: origin.host(),
            image,
            url: input.data,
        },
        &[],
    )
}
