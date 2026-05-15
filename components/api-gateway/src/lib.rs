mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::wasi::http::incoming_handler::{Guest, IncomingRequest, ResponseOutparam};
use bindings::petclinic::owner_aggregate::owner_aggregate as owner_agg;
use bindings::petclinic::pet_aggregate::pet_aggregate as pet_agg;
use bindings::petclinic::vet_aggregate::vet_aggregate as vet_agg;
use bindings::petclinic::gateway::api as query;
use bindings::petclinic::store::event_store;
use bindings::wasi::http::types::{
    Fields, IncomingBody, OutgoingBody, OutgoingResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

struct Component;

export!(Component);

impl Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let path_and_query = request.path_with_query().unwrap_or_default();
        let path = path_and_query.split('?').next().unwrap_or("/").to_string();
        let body = read_body(request.consume().ok());

        use bindings::wasi::http::types::Method;
        let method_str = match method {
            Method::Get => "GET", Method::Post => "POST", Method::Put => "PUT",
            Method::Delete => "DELETE", Method::Patch => "PATCH",
            Method::Head => "HEAD", Method::Options => "OPTIONS",
            Method::Connect => "CONNECT", Method::Trace => "TRACE",
            Method::Other(s) => return send_response(response_out, 405, json_error(&s)),
        };
        let (status, json_body) = route(method_str, &path, body);
        send_response(response_out, status, json_body);
    }
}

fn route(method: &str, path: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();

    match (method, segments.as_slice()) {
        // ── Owners ─────────────────────────────────────────────────────────
        ("POST", ["owners"]) => handle_register_owner(body),
        ("PUT", ["owners", owner_id]) => handle_update_owner(owner_id, body),
        ("POST", ["owners", owner_id, "pets"]) => handle_add_pet(owner_id, body),
        ("GET", ["owners"]) => handle_list_owners(),
        ("GET", ["owners", owner_id]) => handle_get_owner(owner_id),
        ("GET", ["owners", "search"]) => {
            // last-name search via query param — simplified: parse from body
            handle_search_owners(body)
        }

        // ── Vets ───────────────────────────────────────────────────────────
        ("POST", ["vets"]) => handle_register_vet(body),
        ("POST", ["vets", vet_id, "specialties"]) => handle_add_specialty(vet_id, body),
        ("GET", ["vets"]) => handle_list_vets(),
        ("GET", ["vets", vet_id]) => handle_get_vet(vet_id),

        // ── Pets ───────────────────────────────────────────────────────────
        ("POST", ["pets"]) => handle_register_pet(body),
        ("POST", ["pets", pet_id, "visits"]) => handle_schedule_visit(pet_id, body),

        _ => (404, json_error("not found")),
    }
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

fn handle_get_owner(owner_id: &str) -> (u16, Vec<u8>) {
    match query::get_owner(owner_id) {
        Ok(view) => (200, serde_json::to_vec(&view_to_json_owner(view)).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_list_owners() -> (u16, Vec<u8>) {
    match query::list_owners(1, 50) {
        Ok(items) => (200, serde_json::to_vec(&items.iter().map(owner_list_to_json).collect::<Vec<_>>()).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_search_owners(body: Vec<u8>) -> (u16, Vec<u8>) {
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

fn handle_register_vet(body: Vec<u8>) -> (u16, Vec<u8>) {
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

fn handle_get_vet(vet_id: &str) -> (u16, Vec<u8>) {
    match query::get_vet(vet_id) {
        Ok(view) => (200, serde_json::to_vec(&view_to_json_vet(view)).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

fn handle_list_vets() -> (u16, Vec<u8>) {
    match query::list_vets(1, 50) {
        Ok(items) => (200, serde_json::to_vec(&items.iter().map(vet_list_to_json).collect::<Vec<_>>()).unwrap()),
        Err(e) => domain_error_to_http(e),
    }
}

// ── Pet handlers ──────────────────────────────────────────────────────────────

fn handle_register_pet(body: Vec<u8>) -> (u16, Vec<u8>) {
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

fn handle_schedule_visit(pet_id: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
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

// ── Command runners (load history → call aggregate → append events) ────────

fn run_owner_command(
    owner_id: &str,
    cmd: query::OwnerCommand,
) -> Result<(), event_store::StoreError> {
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

fn run_pet_command(
    pet_id: &str,
    cmd: query::PetCommand,
) -> Result<(), event_store::StoreError> {
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

fn run_vet_command(
    vet_id: &str,
    cmd: query::VetCommand,
) -> Result<(), event_store::StoreError> {
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

// ── HTTP helpers ──────────────────────────────────────────────────────────────

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
    let _ = headers.set(
        &"content-type".to_string(),
        &[b"application/json".to_vec()],
    );
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
