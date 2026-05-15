mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::petclinic::vet_aggregate::vet_aggregate::{
    AddSpecialtyCmd, DomainError, Guest, RegisterVetCmd, StoredEvent, VetCommand, VetState,
};
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

impl Guest for Component {
    fn handle_command(
        aggregate_id: String,
        command: VetCommand,
        history: Vec<StoredEvent>,
    ) -> Result<Vec<StoredEvent>, DomainError> {
        let state = Self::reconstruct_state(history)?;
        match command {
            VetCommand::Register(cmd) => handle_register(&aggregate_id, cmd, state),
            VetCommand::AddSpecialty(cmd) => handle_add_specialty(cmd, state),
        }
    }

    fn apply_event(
        state: Option<VetState>,
        event: StoredEvent,
    ) -> Result<VetState, DomainError> {
        apply_vet_event(state, &event)
    }

    fn reconstruct_state(
        history: Vec<StoredEvent>,
    ) -> Result<Option<VetState>, DomainError> {
        history
            .into_iter()
            .try_fold(None, |acc, ev| apply_vet_event(acc, &ev).map(Some))
    }
}

fn handle_register(
    aggregate_id: &str,
    cmd: RegisterVetCmd,
    state: Option<VetState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    if state.is_some() {
        return Err(DomainError::Conflict(format!(
            "vet {aggregate_id} already exists"
        )));
    }
    if cmd.first_name.trim().is_empty() || cmd.last_name.trim().is_empty() {
        return Err(DomainError::Validation("vet name required".into()));
    }
    let vet_id = aggregate_id.to_string();
    let ev = VetEventDto::Registered(VetRegisteredDto {
        vet_id: vet_id.clone(),
        first_name: cmd.first_name,
        last_name: cmd.last_name,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event("vet", &vet_id, 1, "VetRegistered", &ev)?])
}

fn handle_add_specialty(
    cmd: AddSpecialtyCmd,
    state: Option<VetState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    let state = state.ok_or_else(|| DomainError::NotFound(cmd.vet_id.clone()))?;
    let already = state.specialties.iter().any(|s| s.id == cmd.specialty.id);
    if already {
        return Err(DomainError::Conflict(format!(
            "specialty {} already assigned",
            cmd.specialty.id
        )));
    }
    let next_seq = state.version + 1;
    let ev = VetEventDto::SpecialtyAdded(SpecialtyAddedDto {
        vet_id: cmd.vet_id.clone(),
        specialty_id: cmd.specialty.id.clone(),
        specialty_name: cmd.specialty.name.clone(),
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event(
        "vet",
        &cmd.vet_id,
        next_seq,
        "SpecialtyAdded",
        &ev,
    )?])
}

fn apply_vet_event(
    state: Option<VetState>,
    event: &StoredEvent,
) -> Result<VetState, DomainError> {
    let payload = String::from_utf8(event.payload_json.clone())
        .map_err(|e| DomainError::Internal(e.to_string()))?;
    let ev: VetEventDto = serde_json::from_str(&payload)
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match ev {
        VetEventDto::Registered(r) => Ok(VetState {
            vet_id: r.vet_id,
            first_name: r.first_name,
            last_name: r.last_name,
            specialties: vec![],
            version: event.sequence,
        }),
        VetEventDto::SpecialtyAdded(s) => {
            let mut st = state.ok_or_else(|| {
                DomainError::Internal("SpecialtyAdded without prior vet state".into())
            })?;
            st.specialties.push(exports::petclinic::vet_aggregate::vet_aggregate::SpecialtyRef {
                id: s.specialty_id,
                name: s.specialty_name,
            });
            st.version = event.sequence;
            Ok(st)
        }
    }
}

fn new_id() -> String { uuid::Uuid::now_v7().to_string() }

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn make_stored_event(
    aggregate_type: &str,
    aggregate_id: &str,
    sequence: u64,
    event_type: &str,
    payload: &impl Serialize,
) -> Result<StoredEvent, DomainError> {
    let json = serde_json::to_string(payload)
        .map_err(|e| DomainError::Internal(e.to_string()))?;
    Ok(StoredEvent {
        event_id: new_id(),
        aggregate_type: aggregate_type.to_string(),
        aggregate_id: aggregate_id.to_string(),
        sequence,
        event_type: event_type.to_string(),
        payload_json: json.into_bytes(),
        occurred_at_ms: now_ms(),
    })
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum VetEventDto {
    Registered(VetRegisteredDto),
    SpecialtyAdded(SpecialtyAddedDto),
}

#[derive(Serialize, Deserialize)]
struct VetRegisteredDto {
    vet_id:      String,
    first_name:  String,
    last_name:   String,
    occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct SpecialtyAddedDto {
    vet_id:          String,
    specialty_id:    String,
    specialty_name:  String,
    occurred_at:     u64,
}
