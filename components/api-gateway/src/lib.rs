mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::wasi::http::incoming_handler::{Guest, IncomingRequest, ResponseOutparam};
use bindings::petclinic::owner_aggregate::owner_aggregate as owner_agg;
use bindings::petclinic::pet_aggregate::pet_aggregate as pet_agg;
use bindings::petclinic::vet_aggregate::vet_aggregate as vet_agg;
use bindings::petclinic::gateway::api as query;
use bindings::petclinic::store::event_store;
use bindings::wasi::http::types::{Fields, IncomingBody, OutgoingBody, OutgoingResponse};
use bindings::wasi::keyvalue::store as kv;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Sha256, Digest};
use hmac::{Hmac, Mac};

type HmacSha256 = Hmac<Sha256>;

const JWT_SECRET: &str = "petclinic-jwt-secret-change-in-prod";
const KV_BUCKET: &str = "default";

struct Component;

export!(Component);

impl Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let path_and_query = request.path_with_query().unwrap_or_default();
        let path = path_and_query.split('?').next().unwrap_or("/").to_string();
        let token = extract_bearer_token(&request);
        let body = read_body(request.consume().ok());

        use bindings::wasi::http::types::Method;
        let method_str = match method {
            Method::Get => "GET", Method::Post => "POST", Method::Put => "PUT",
            Method::Delete => "DELETE", Method::Patch => "PATCH",
            Method::Head => "HEAD", Method::Options => "OPTIONS",
            Method::Connect => "CONNECT", Method::Trace => "TRACE",
            Method::Other(s) => return send_response(response_out, 405, json_error(&s)),
        };
        let (status, json_body) = route(method_str, &path, body, &token);
        send_response(response_out, status, json_body);
    }
}

fn route(method: &str, path: &str, body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    match (method, segments.as_slice()) {
        // ── Auth ───────────────────────────────────────────────────────────
        ("POST", ["auth", "register-owner"]) => handle_auth_register_owner(body),
        ("POST", ["auth", "register-vet"])   => handle_auth_register_vet(body),
        ("POST", ["auth", "login"])          => handle_auth_login(body),

        // ── Admin ──────────────────────────────────────────────────────────
        ("POST", ["admin", "bootstrap"]) => handle_admin_bootstrap(body),
        ("POST", ["admin", "invites"])   => handle_admin_create_invite(body, token),
        ("GET",  ["admin", "invites"])   => handle_admin_list_invites(token),

        // ── Owners ─────────────────────────────────────────────────────────
        ("POST", ["owners"])                       => handle_register_owner(body),
        ("PUT",  ["owners", owner_id])             => handle_update_owner(owner_id, body),
        ("POST", ["owners", owner_id, "pets"])     => handle_add_pet(owner_id, body),
        ("GET",  ["owners"])                       => handle_list_owners(token),
        ("GET",  ["owners", "search"])             => handle_search_owners(body, token),
        ("GET",  ["owners", owner_id])             => handle_get_owner(owner_id, token),

        // ── Vets ───────────────────────────────────────────────────────────
        ("POST", ["vets"])                         => handle_register_vet(body, token),
        ("POST", ["vets", vet_id, "specialties"])  => handle_add_specialty(vet_id, body),
        ("GET",  ["vets"])                         => handle_list_vets(token),
        ("GET",  ["vets", vet_id])                 => handle_get_vet(vet_id, token),

        // ── Pets ───────────────────────────────────────────────────────────
        ("POST", ["pets"])                   => handle_register_pet(body, token),
        ("POST", ["pets", pet_id, "visits"]) => handle_schedule_visit(pet_id, body, token),

        // ── Medical documents ──────────────────────────────────────────────
        ("POST", ["pets", pet_id, "medical-documents"]) => handle_upload_medical_doc(pet_id, body, token),
        ("GET",  ["pets", pet_id, "medical-documents"]) => handle_list_medical_docs(pet_id, token),

        // ── CORS preflight ─────────────────────────────────────────────────
        ("OPTIONS", _) => (204, Vec::new()),

        _ => (404, json_error("not found")),
    }
}

// ── Auth handlers ─────────────────────────────────────────────────────────────

fn handle_auth_register_owner(body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        username: String, password: String,
        first_name: String, last_name: String,
        address: String, city: String, telephone: String,
    }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    if req.username.is_empty() || req.password.is_empty() {
        return (400, json_error("username and password required"));
    }
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let user_key = format!("user.{}", req.username);
    if let Ok(Some(_)) = bucket.get(&user_key) {
        return (409, json_error("username already exists"));
    }
    let owner_id = new_id();
    let cmd = query::OwnerCommand::Register(query::RegisterOwnerCmd {
        idempotency_key: owner_id.clone(),
        first_name: req.first_name, last_name: req.last_name,
        address: req.address, city: req.city, telephone: req.telephone,
    });
    if let Err(e) = run_owner_command(&owner_id, cmd) {
        return store_error_to_http(e);
    }
    let user_id = new_id();
    let salt = new_id();
    let password_hash = sha256_hex(format!("{}:{}", salt, req.password).as_bytes());
    let record = UserRecord {
        user_id: user_id.clone(),
        username: req.username,
        password_hash, salt,
        role: "owner".to_string(),
        entity_id: owner_id.clone(),
    };
    if let Err(e) = bucket.set(&user_key, &serde_json::to_vec(&record).unwrap()) {
        return (500, json_error(&format!("{e:?}")));
    }
    let now = bindings::wasi::clocks::wall_clock::now();
    let token = make_jwt(&user_id, "owner", &owner_id, now.seconds);
    (201, serde_json::to_vec(&serde_json::json!({
        "token":   token,
        "userId":  user_id,
        "ownerId": owner_id,
        "role":    "owner",
        "entityId": owner_id,
    })).unwrap())
}

fn handle_auth_register_vet(body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        invite_token: String,
        username: String, password: String,
        first_name: String, last_name: String,
    }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let invite_key = format!("invite.{}", req.invite_token);
    let invite: InviteRecord = match bucket.get(&invite_key) {
        Ok(Some(b)) => match serde_json::from_slice(&b) {
            Ok(i) => i, Err(_) => return (500, json_error("corrupt invite record")),
        },
        Ok(None) => return (404, json_error("invite not found")),
        Err(e)   => return (500, json_error(&format!("{e:?}"))),
    };
    if invite.used {
        return (409, json_error("invite already used"));
    }
    let now = bindings::wasi::clocks::wall_clock::now();
    if invite.expires_at <= now.seconds {
        return (410, json_error("invite expired"));
    }
    let user_key = format!("user.{}", req.username);
    if let Ok(Some(_)) = bucket.get(&user_key) {
        return (409, json_error("username already exists"));
    }
    let vet_id = new_id();
    let cmd = query::VetCommand::Register(query::RegisterVetCmd {
        idempotency_key: vet_id.clone(),
        first_name: req.first_name, last_name: req.last_name,
    });
    if let Err(e) = run_vet_command(&vet_id, cmd) {
        return store_error_to_http(e);
    }
    let user_id = new_id();
    let salt = new_id();
    let password_hash = sha256_hex(format!("{}:{}", salt, req.password).as_bytes());
    let record = UserRecord {
        user_id: user_id.clone(),
        username: req.username,
        password_hash, salt,
        role: "vet".to_string(),
        entity_id: vet_id.clone(),
    };
    if let Err(e) = bucket.set(&user_key, &serde_json::to_vec(&record).unwrap()) {
        return (500, json_error(&format!("{e:?}")));
    }
    let used_invite = InviteRecord { used: true, ..invite };
    let _ = bucket.set(&invite_key, &serde_json::to_vec(&used_invite).unwrap());
    let token = make_jwt(&user_id, "vet", &vet_id, now.seconds);
    (201, serde_json::to_vec(&serde_json::json!({
        "token":  token,
        "userId": user_id,
        "vetId":  vet_id,
        "role":   "vet",
        "entityId": vet_id,
    })).unwrap())
}

fn handle_auth_login(body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    struct Req { username: String, password: String }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let user_key = format!("user.{}", req.username);
    let record: UserRecord = match bucket.get(&user_key) {
        Ok(Some(b)) => match serde_json::from_slice(&b) {
            Ok(r) => r, Err(_) => return (500, json_error("corrupt user record")),
        },
        Ok(None) => return (401, json_error("invalid credentials")),
        Err(e)   => return (500, json_error(&format!("{e:?}"))),
    };
    let expected = sha256_hex(format!("{}:{}", record.salt, req.password).as_bytes());
    if expected != record.password_hash {
        return (401, json_error("invalid credentials"));
    }
    let now = bindings::wasi::clocks::wall_clock::now();
    let token = make_jwt(&record.user_id, &record.role, &record.entity_id, now.seconds);
    (200, serde_json::to_vec(&serde_json::json!({
        "token":    token,
        "userId":   record.user_id,
        "role":     record.role,
        "entityId": record.entity_id,
    })).unwrap())
}

// ── Admin handlers ────────────────────────────────────────────────────────────

fn handle_admin_bootstrap(body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    struct Req { username: String, password: String }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    if req.username.is_empty() || req.password.is_empty() {
        return (400, json_error("username and password required"));
    }
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    if let Ok(Some(_)) = bucket.get("admin.bootstrapped") {
        return (409, json_error("admin already bootstrapped"));
    }
    let user_key = format!("user.{}", req.username);
    if let Ok(Some(_)) = bucket.get(&user_key) {
        return (409, json_error("username already exists"));
    }
    let user_id = new_id();
    let salt = new_id();
    let password_hash = sha256_hex(format!("{}:{}", salt, req.password).as_bytes());
    let record = UserRecord {
        user_id: user_id.clone(),
        username: req.username,
        password_hash, salt,
        role: "admin".to_string(),
        entity_id: user_id.clone(),
    };
    if let Err(e) = bucket.set(&user_key, &serde_json::to_vec(&record).unwrap()) {
        return (500, json_error(&format!("{e:?}")));
    }
    if let Err(e) = bucket.set("admin.bootstrapped", b"true") {
        return (500, json_error(&format!("{e:?}")));
    }
    let now = bindings::wasi::clocks::wall_clock::now();
    let token = make_jwt(&user_id, "admin", &user_id, now.seconds);
    (201, serde_json::to_vec(&serde_json::json!({
        "token":  token,
        "userId": user_id,
        "role":   "admin",
    })).unwrap())
}

fn handle_admin_create_invite(_body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "admin" {
        return (403, json_error("admin only"));
    }
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let now = bindings::wasi::clocks::wall_clock::now();
    let token_id = new_id();
    let invite = InviteRecord {
        token: token_id.clone(),
        created_at: now.seconds,
        expires_at: now.seconds + 72 * 3600,
        used: false,
    };
    let invite_key = format!("invite.{token_id}");
    if let Err(e) = bucket.set(&invite_key, &serde_json::to_vec(&invite).unwrap()) {
        return (500, json_error(&format!("{e:?}")));
    }
    let mut ids: Vec<String> = bucket.get("invite.idx")
        .ok().flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    ids.push(token_id.clone());
    let _ = bucket.set("invite.idx", &serde_json::to_vec(&ids).unwrap());
    (201, serde_json::to_vec(&serde_json::json!({
        "token":     token_id,
        "link":      format!("/#invite/{token_id}"),
        "expiresAt": invite.expires_at,
    })).unwrap())
}

fn handle_admin_list_invites(token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "admin" {
        return (403, json_error("admin only"));
    }
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let ids: Vec<String> = bucket.get("invite.idx")
        .ok().flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut invites = Vec::new();
    for id in &ids {
        if let Ok(Some(bytes)) = bucket.get(&format!("invite.{id}")) {
            if let Ok(inv) = serde_json::from_slice::<InviteRecord>(&bytes) {
                invites.push(serde_json::json!({
                    "token":     inv.token,
                    "createdAt": inv.created_at,
                    "expiresAt": inv.expires_at,
                    "used":      inv.used,
                }));
            }
        }
    }
    (200, serde_json::to_vec(&invites).unwrap())
}

// ── Medical document handlers ─────────────────────────────────────────────────

fn handle_upload_medical_doc(pet_id: &str, body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "vet" {
        return (403, json_error("only vets can upload medical documents"));
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AttachmentReq { filename: String, content_type: String, data_base64: String }
    #[derive(Deserialize)]
    struct Req { title: String, notes: String, attachments: Vec<AttachmentReq> }

    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let now = bindings::wasi::clocks::wall_clock::now();
    let doc_id = new_id();

    for (i, att) in req.attachments.iter().enumerate() {
        let blob_key = format!("medoc.blob.{doc_id}.{i}");
        let att_data = serde_json::to_vec(&serde_json::json!({
            "filename":    att.filename,
            "contentType": att.content_type,
            "dataBase64":  att.data_base64,
        })).unwrap();
        if let Err(e) = bucket.set(&blob_key, &att_data) {
            return (500, json_error(&format!("blob store failed: {e:?}")));
        }
    }

    let doc = MedicalDocRecord {
        doc_id: doc_id.clone(),
        pet_id: pet_id.to_string(),
        vet_id: claims.eid.clone(),
        title: req.title,
        notes: req.notes,
        created_at: now.seconds,
        attachment_count: req.attachments.len() as u32,
    };
    if let Err(e) = bucket.set(&format!("medoc.{doc_id}"), &serde_json::to_vec(&doc).unwrap()) {
        return (500, json_error(&format!("{e:?}")));
    }

    let idx_key = format!("medoc.idx.pet.{pet_id}");
    let mut ids: Vec<String> = bucket.get(&idx_key)
        .ok().flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    ids.push(doc_id.clone());
    let _ = bucket.set(&idx_key, &serde_json::to_vec(&ids).unwrap());

    (201, serde_json::to_vec(&serde_json::json!({ "docId": doc_id })).unwrap())
}

fn handle_list_medical_docs(pet_id: &str, token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    let bucket = match kv::open(KV_BUCKET) {
        Ok(b) => b, Err(e) => return (500, json_error(&format!("{e:?}"))),
    };
    let idx_key = format!("medoc.idx.pet.{pet_id}");
    let ids: Vec<String> = bucket.get(&idx_key)
        .ok().flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let mut docs = Vec::new();
    for id in &ids {
        if let Ok(Some(bytes)) = bucket.get(&format!("medoc.{id}")) {
            if let Ok(doc) = serde_json::from_slice::<MedicalDocRecord>(&bytes) {
                docs.push(serde_json::json!({
                    "docId":           doc.doc_id,
                    "petId":           doc.pet_id,
                    "vetId":           doc.vet_id,
                    "title":           doc.title,
                    "notes":           doc.notes,
                    "attachmentCount": doc.attachment_count,
                    "createdAt":       doc.created_at,
                }));
            }
        }
    }
    (200, serde_json::to_vec(&docs).unwrap())
}

// ── Owner handlers ────────────────────────────────────────────────────────────

fn handle_register_owner(body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req { first_name: String, last_name: String, address: String, city: String, telephone: String }

    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return (400, json_error(&e.to_string())),
    };

    let owner_id = new_id();
    let cmd = query::OwnerCommand::Register(query::RegisterOwnerCmd {
        idempotency_key: owner_id.clone(),
        first_name: req.first_name, last_name: req.last_name,
        address: req.address, city: req.city, telephone: req.telephone,
    });

    match run_owner_command(&owner_id, cmd) {
        Ok(_) => (201, serde_json::to_vec(&serde_json::json!({ "ownerId": owner_id })).unwrap()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_update_owner(owner_id: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        first_name: Option<String>, last_name: Option<String>,
        address: Option<String>, city: Option<String>, telephone: Option<String>,
    }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    let cmd = query::OwnerCommand::Update(query::UpdateOwnerCmd {
        owner_id: owner_id.to_string(),
        first_name: req.first_name, last_name: req.last_name,
        address: req.address, city: req.city, telephone: req.telephone,
    });
    match run_owner_command(owner_id, cmd) {
        Ok(_) => (200, json_ok()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_add_pet(owner_id: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req {
        name: String, birth_date: String,
        pet_type_id: String, pet_type_name: String,
    }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    let cmd = query::OwnerCommand::AddPet(query::AddPetCmd {
        owner_id: owner_id.to_string(), name: req.name, birth_date: req.birth_date,
        pet_type: query::PetTypeRef {
            id: req.pet_type_id, name: req.pet_type_name,
        },
    });
    match run_owner_command(owner_id, cmd) {
        Ok(_) => (201, json_ok()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_get_owner(owner_id: &str, token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    match query::get_owner(owner_id) {
        Ok(view) => (200, serde_json::to_vec(&view_to_json_owner(view)).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_list_owners(token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    match query::list_owners(1, 50) {
        Ok(items) => (200, serde_json::to_vec(&items.iter().map(owner_list_to_json).collect::<Vec<_>>()).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_search_owners(body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    #[derive(Deserialize)]
    struct Req { last_name: String }
    let req: Req = match serde_json::from_slice(&body) {
        Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())),
    };
    match query::search_owners_by_last_name(&req.last_name) {
        Ok(items) => (200, serde_json::to_vec(&items.iter().map(owner_list_to_json).collect::<Vec<_>>()).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

// ── Vet handlers ──────────────────────────────────────────────────────────────

fn handle_register_vet(body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "admin" {
        return (403, json_error("admin only"));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req { first_name: String, last_name: String }
    let req: Req = match serde_json::from_slice(&body) { Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())) };
    let vet_id = new_id();
    let cmd = query::VetCommand::Register(query::RegisterVetCmd {
        idempotency_key: vet_id.clone(), first_name: req.first_name, last_name: req.last_name,
    });
    match run_vet_command(&vet_id, cmd) {
        Ok(_) => (201, serde_json::to_vec(&serde_json::json!({ "vetId": vet_id })).unwrap()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_add_specialty(vet_id: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req { specialty_id: String, specialty_name: String }
    let req: Req = match serde_json::from_slice(&body) { Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())) };
    let cmd = query::VetCommand::AddSpecialty(query::AddSpecialtyCmd {
        vet_id: vet_id.to_string(),
        specialty: query::SpecialtyRef { id: req.specialty_id, name: req.specialty_name },
    });
    match run_vet_command(vet_id, cmd) {
        Ok(_) => (200, json_ok()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_get_vet(vet_id: &str, token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    match query::get_vet(vet_id) {
        Ok(view) => (200, serde_json::to_vec(&view_to_json_vet(view)).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_list_vets(token: &str) -> (u16, Vec<u8>) {
    if validate_jwt(token).is_none() {
        return (401, json_error("unauthorized"));
    }
    match query::list_vets(1, 50) {
        Ok(items) => (200, serde_json::to_vec(&items.iter().map(vet_list_to_json).collect::<Vec<_>>()).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

// ── Pet handlers ──────────────────────────────────────────────────────────────

fn handle_register_pet(body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "owner" {
        return (403, json_error("owner only"));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Req { owner_id: String, name: String, birth_date: String, pet_type_id: String, pet_type_name: String }
    let req: Req = match serde_json::from_slice(&body) { Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())) };
    let pet_id = new_id();
    let cmd = query::PetCommand::Register(query::RegisterPetCmd {
        idempotency_key: pet_id.clone(), owner_id: req.owner_id,
        name: req.name, birth_date: req.birth_date,
        pet_type: query::PetTypeRef { id: req.pet_type_id, name: req.pet_type_name },
    });
    match run_pet_command(&pet_id, cmd) {
        Ok(_) => (201, serde_json::to_vec(&serde_json::json!({ "petId": pet_id })).unwrap()),
        Err(e) => store_error_to_http(e),
    }
}

fn handle_schedule_visit(pet_id: &str, body: Vec<u8>, token: &str) -> (u16, Vec<u8>) {
    let claims = match validate_jwt(token) {
        Some(c) => c, None => return (401, json_error("unauthorized")),
    };
    if claims.role != "vet" {
        return (403, json_error("vet only"));
    }
    #[derive(Deserialize)]
    struct Req { date: String, description: String }
    let req: Req = match serde_json::from_slice(&body) { Ok(r) => r, Err(e) => return (400, json_error(&e.to_string())) };
    let cmd = query::PetCommand::ScheduleVisit(query::ScheduleVisitCmd {
        pet_id: pet_id.to_string(), date: req.date, description: req.description,
    });
    match run_pet_command(pet_id, cmd) {
        Ok(_) => (201, json_ok()),
        Err(e) => store_error_to_http(e),
    }
}

// ── Command runners ────────────────────────────────────────────────────────────

fn run_owner_command(owner_id: &str, cmd: query::OwnerCommand) -> Result<(), event_store::StoreError> {
    let store_history = event_store::load_events("owner", owner_id)?;
    let current_seq = store_history.last().map(|e| e.sequence).unwrap_or(0);
    let history: Vec<_> = store_history.into_iter().map(from_store_event).collect();
    let agg_cmd = convert_owner_command(cmd);
    let new_events: Vec<_> = owner_agg::handle_command(owner_id, &agg_cmd, &history)
        .map_err(|e| event_store::StoreError::Internal(format!("{e:?}")))?
        .into_iter().map(to_store_event).collect();
    event_store::append_events("owner", owner_id, current_seq, &new_events)?;
    Ok(())
}

fn run_pet_command(pet_id: &str, cmd: query::PetCommand) -> Result<(), event_store::StoreError> {
    let store_history = event_store::load_events("pet", pet_id)?;
    let current_seq = store_history.last().map(|e| e.sequence).unwrap_or(0);
    let history: Vec<_> = store_history.into_iter().map(from_store_event_pet).collect();
    let agg_cmd = convert_pet_command(cmd);
    let new_events: Vec<_> = pet_agg::handle_command(pet_id, &agg_cmd, &history)
        .map_err(|e| event_store::StoreError::Internal(format!("{e:?}")))?
        .into_iter().map(to_store_event_pet).collect();
    event_store::append_events("pet", pet_id, current_seq, &new_events)?;
    Ok(())
}

fn run_vet_command(vet_id: &str, cmd: query::VetCommand) -> Result<(), event_store::StoreError> {
    let store_history = event_store::load_events("vet", vet_id)?;
    let current_seq = store_history.last().map(|e| e.sequence).unwrap_or(0);
    let history: Vec<_> = store_history.into_iter().map(from_store_event_vet).collect();
    let agg_cmd = convert_vet_command(cmd);
    let new_events: Vec<_> = vet_agg::handle_command(vet_id, &agg_cmd, &history)
        .map_err(|e| event_store::StoreError::Internal(format!("{e:?}")))?
        .into_iter().map(to_store_event_vet).collect();
    event_store::append_events("vet", vet_id, current_seq, &new_events)?;
    Ok(())
}

fn convert_owner_command(cmd: query::OwnerCommand) -> owner_agg::OwnerCommand {
    match cmd {
        query::OwnerCommand::Register(c) => owner_agg::OwnerCommand::Register(owner_agg::RegisterOwnerCmd {
            idempotency_key: c.idempotency_key,
            first_name: c.first_name, last_name: c.last_name,
            address: c.address, city: c.city, telephone: c.telephone,
        }),
        query::OwnerCommand::Update(c) => owner_agg::OwnerCommand::Update(owner_agg::UpdateOwnerCmd {
            owner_id: c.owner_id,
            first_name: c.first_name, last_name: c.last_name,
            address: c.address, city: c.city, telephone: c.telephone,
        }),
        query::OwnerCommand::AddPet(c) => owner_agg::OwnerCommand::AddPet(owner_agg::AddPetCmd {
            owner_id: c.owner_id, name: c.name, birth_date: c.birth_date,
            pet_type: owner_agg::PetTypeRef { id: c.pet_type.id, name: c.pet_type.name },
        }),
    }
}

fn convert_pet_command(cmd: query::PetCommand) -> pet_agg::PetCommand {
    match cmd {
        query::PetCommand::Register(c) => pet_agg::PetCommand::Register(pet_agg::RegisterPetCmd {
            idempotency_key: c.idempotency_key,
            owner_id: c.owner_id, name: c.name, birth_date: c.birth_date,
            pet_type: pet_agg::PetTypeRef { id: c.pet_type.id, name: c.pet_type.name },
        }),
        query::PetCommand::ScheduleVisit(c) => pet_agg::PetCommand::ScheduleVisit(pet_agg::ScheduleVisitCmd {
            pet_id: c.pet_id, date: c.date, description: c.description,
        }),
    }
}

fn convert_vet_command(cmd: query::VetCommand) -> vet_agg::VetCommand {
    match cmd {
        query::VetCommand::Register(c) => vet_agg::VetCommand::Register(vet_agg::RegisterVetCmd {
            idempotency_key: c.idempotency_key,
            first_name: c.first_name, last_name: c.last_name,
        }),
        query::VetCommand::AddSpecialty(c) => vet_agg::VetCommand::AddSpecialty(vet_agg::AddSpecialtyCmd {
            vet_id: c.vet_id,
            specialty: vet_agg::SpecialtyRef { id: c.specialty.id, name: c.specialty.name },
        }),
    }
}

// ── KV data types ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct UserRecord {
    user_id:       String,
    username:      String,
    password_hash: String,
    salt:          String,
    role:          String,
    entity_id:     String,
}

#[derive(Serialize, Deserialize)]
struct MedicalDocRecord {
    doc_id:           String,
    pet_id:           String,
    vet_id:           String,
    title:            String,
    notes:            String,
    created_at:       u64,
    attachment_count: u32,
}

#[derive(Serialize, Deserialize)]
struct InviteRecord {
    token:      String,
    created_at: u64,
    expires_at: u64,
    used:       bool,
}

// ── JWT ───────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct JwtClaims {
    sub:  String,
    role: String,
    eid:  String,
    exp:  u64,
}

fn make_jwt(user_id: &str, role: &str, entity_id: &str, now_secs: u64) -> String {
    let header  = base64url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
    let payload = base64url_encode(
        serde_json::to_string(&JwtClaims {
            sub: user_id.to_string(),
            role: role.to_string(),
            eid: entity_id.to_string(),
            exp: now_secs + 86400,
        }).unwrap().as_bytes(),
    );
    let signing_input = format!("{header}.{payload}");
    let sig = base64url_encode(&hmac_sha256(JWT_SECRET.as_bytes(), signing_input.as_bytes()));
    format!("{signing_input}.{sig}")
}

fn validate_jwt(token: &str) -> Option<JwtClaims> {
    if token.is_empty() { return None; }
    let mut parts = token.splitn(3, '.');
    let header  = parts.next()?;
    let payload = parts.next()?;
    let sig     = parts.next()?;
    let signing_input = format!("{header}.{payload}");
    let expected_sig = base64url_encode(&hmac_sha256(JWT_SECRET.as_bytes(), signing_input.as_bytes()));
    if expected_sig != sig { return None; }
    let payload_bytes = base64url_decode(payload)?;
    let claims: JwtClaims = serde_json::from_slice(&payload_bytes).ok()?;
    let now = bindings::wasi::clocks::wall_clock::now();
    if claims.exp <= now.seconds { return None; }
    Some(claims)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn base64url_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() * 4 + 2) / 3);
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        out.push(T[((b0 >> 2) & 0x3F) as usize] as char);
        out.push(T[(((b0 << 4) | (b1 >> 4)) & 0x3F) as usize] as char);
        if i + 1 < data.len() { out.push(T[(((b1 << 2) | (b2 >> 6)) & 0x3F) as usize] as char); }
        if i + 2 < data.len() { out.push(T[(b2 & 0x3F) as usize] as char); }
        i += 3;
    }
    out
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'-'        => Some(62),
            b'_'        => Some(63),
            _           => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    let mut i = 0;
    while i < b.len() {
        let v0 = val(b[i])?;
        let v1 = if i + 1 < b.len() { val(b[i + 1])? } else { break };
        out.push((v0 << 2) | (v1 >> 4));
        if i + 2 < b.len() {
            let v2 = val(b[i + 2])?;
            out.push(((v1 & 0xF) << 4) | (v2 >> 2));
            if i + 3 < b.len() {
                let v3 = val(b[i + 3])?;
                out.push(((v2 & 0x3) << 6) | v3);
            }
        }
        i += 4;
    }
    Some(out)
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

fn extract_bearer_token(request: &IncomingRequest) -> String {
    for val_bytes in request.headers().get(&"authorization".to_string()) {
        if let Ok(s) = String::from_utf8(val_bytes) {
            if let Some(tok) = s.strip_prefix("Bearer ") {
                return tok.to_string();
            }
        }
    }
    String::new()
}

fn read_body(body: Option<IncomingBody>) -> Vec<u8> {
    let Some(b) = body else { return Vec::new() };
    let Ok(stream) = b.stream() else {
        IncomingBody::finish(b);
        return Vec::new();
    };
    let mut data = Vec::new();
    loop {
        match stream.blocking_read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => data.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    IncomingBody::finish(b);
    data
}

fn send_response(out: ResponseOutparam, status: u16, body: Vec<u8>) {
    let headers = Fields::new();
    let _ = headers.set(&"content-type".to_string(),                   &[b"application/json".to_vec()]);
    let _ = headers.set(&"access-control-allow-origin".to_string(),    &[b"*".to_vec()]);
    let _ = headers.set(&"access-control-allow-headers".to_string(),   &[b"Content-Type, Authorization".to_vec()]);
    let _ = headers.set(&"access-control-allow-methods".to_string(),   &[b"GET, POST, PUT, DELETE, OPTIONS".to_vec()]);
    let _ = headers.set(&"access-control-expose-headers".to_string(),  &[b"*".to_vec()]);
    let resp = OutgoingResponse::new(headers);
    resp.set_status_code(status).ok();
    let ob = resp.body().expect("body");
    ResponseOutparam::set(out, Ok(resp));
    if let Ok(stream) = ob.write() {
        stream.blocking_write_and_flush(&body).ok();
        drop(stream);
    }
    OutgoingBody::finish(ob, None).ok();
}

fn to_store_event(e: owner_agg::StoredEvent) -> event_store::StoredEvent {
    event_store::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}
fn to_store_event_pet(e: pet_agg::StoredEvent) -> event_store::StoredEvent {
    event_store::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}
fn to_store_event_vet(e: vet_agg::StoredEvent) -> event_store::StoredEvent {
    event_store::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}
fn from_store_event(e: event_store::StoredEvent) -> owner_agg::StoredEvent {
    owner_agg::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}
fn from_store_event_pet(e: event_store::StoredEvent) -> pet_agg::StoredEvent {
    pet_agg::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}
fn from_store_event_vet(e: event_store::StoredEvent) -> vet_agg::StoredEvent {
    vet_agg::StoredEvent {
        event_id: e.event_id, aggregate_type: e.aggregate_type, aggregate_id: e.aggregate_id,
        sequence: e.sequence, event_type: e.event_type, payload_json: e.payload_json,
        occurred_at_ms: e.occurred_at_ms,
    }
}

fn store_error_to_http(e: event_store::StoreError) -> (u16, Vec<u8>) {
    let (status, msg) = match e {
        event_store::StoreError::NotFound(m)  => (404, m),
        event_store::StoreError::Conflict(m)  => (409, m),
        event_store::StoreError::Backend(m)   => (500, m),
        event_store::StoreError::Internal(m)  => (500, m),
    };
    (status, json_error(&msg))
}

fn domain_error_to_http(e: query::DomainError) -> (u16, Vec<u8>) {
    let (status, msg) = match e {
        query::DomainError::NotFound(m)   => (404, m),
        query::DomainError::Validation(m) => (400, m),
        query::DomainError::Conflict(m)   => (409, m),
        query::DomainError::StoreError(m) => (500, m),
        query::DomainError::Internal(m)   => (500, m),
    };
    (status, json_error(&msg))
}

fn json_error(msg: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "error": msg })).unwrap()
}

fn json_ok() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "ok": true })).unwrap()
}

fn new_id() -> String { uuid::Uuid::now_v7().to_string() }

// ── View → JSON conversions ───────────────────────────────────────────────────

fn view_to_json_owner(v: query::OwnerProfileView) -> Value {
    serde_json::json!({
        "ownerId":   v.owner_id,
        "firstName": v.first_name,
        "lastName":  v.last_name,
        "address":   v.address,
        "city":      v.city,
        "telephone": v.telephone,
        "version":   v.version,
        "pets": v.pets.into_iter().map(|p| serde_json::json!({
            "petId":     p.pet_id,
            "name":      p.name,
            "birthDate": p.birth_date,
            "petType":   { "id": p.pet_type.id, "name": p.pet_type.name },
            "visits": p.visits.into_iter().map(|vis| serde_json::json!({
                "visitId":     vis.visit_id,
                "date":        vis.date,
                "description": vis.description,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn owner_list_to_json(i: &query::OwnerListItem) -> Value {
    serde_json::json!({
        "ownerId":   i.owner_id, "firstName": i.first_name,
        "lastName":  i.last_name, "city": i.city, "petCount": i.pet_count,
    })
}

fn view_to_json_vet(v: query::VetProfileView) -> Value {
    serde_json::json!({
        "vetId":      v.vet_id,
        "firstName":  v.first_name,
        "lastName":   v.last_name,
        "version":    v.version,
        "specialties": v.specialties.into_iter().map(|s| serde_json::json!({
            "id": s.id, "name": s.name,
        })).collect::<Vec<_>>(),
    })
}

fn vet_list_to_json(i: &query::VetListItem) -> Value {
    serde_json::json!({
        "vetId": i.vet_id, "firstName": i.first_name,
        "lastName": i.last_name, "specialtyCount": i.specialty_count,
    })
}
