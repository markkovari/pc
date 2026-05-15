mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::wasi::http::incoming_handler::{Guest, IncomingRequest, ResponseOutparam};
use bindings::wasi::http::types::{Fields, OutgoingBody, OutgoingResponse};

struct Component;

export!(Component);

static INDEX_HTML: &[u8] = include_bytes!("../assets/index.html");
static APP_JS:    &[u8] = include_bytes!("../assets/app.js");

impl Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let path = request
            .path_with_query()
            .unwrap_or_default();
        let path = path.split('?').next().unwrap_or("/");

        let (content_type, body): (&str, &[u8]) = match path {
            "/" | "/index.html" => ("text/html; charset=utf-8", INDEX_HTML),
            "/app.js"           => ("application/javascript", APP_JS),
            _                   => ("text/plain", b"not found"),
        };
        let status: u16 = if body == b"not found" { 404 } else { 200 };

        let headers = Fields::new();
        let _ = headers.set(&"content-type".to_string(), &[content_type.as_bytes().to_vec()]);
        let resp = OutgoingResponse::new(headers);
        resp.set_status_code(status).ok();
        if let Ok(ob) = resp.body() {
            if let Ok(stream) = ob.write() {
                stream.write(body).ok();
            }
            OutgoingBody::finish(ob, None).ok();
        }
        ResponseOutparam::set(response_out, Ok(resp));
    }
}
