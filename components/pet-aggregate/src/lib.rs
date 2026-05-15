mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::petclinic::pet_aggregate::pet_aggregate::{
    DomainError, Guest, PetCommand, PetState, RegisterPetCmd, ScheduleVisitCmd, StoredEvent,
    VisitSummary,
};
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

impl Guest for Component {
    fn handle_command(
        aggregate_id: String,
        command: PetCommand,
        history: Vec<StoredEvent>,
    ) -> Result<Vec<StoredEvent>, DomainError> {
        let state = Self::reconstruct_state(history)?;
        match command {
            PetCommand::Register(cmd) => handle_register(&aggregate_id, cmd, state),
            PetCommand::ScheduleVisit(cmd) => handle_schedule_visit(cmd, state),
        }
    }

    fn apply_event(
        state: Option<PetState>,
        event: StoredEvent,
    ) -> Result<PetState, DomainError> {
        apply_pet_event(state, &event)
    }

    fn reconstruct_state(
        history: Vec<StoredEvent>,
    ) -> Result<Option<PetState>, DomainError> {
        history
            .into_iter()
            .try_fold(None, |acc, ev| apply_pet_event(acc, &ev).map(Some))
    }
}

fn handle_register(
    aggregate_id: &str,
    cmd: RegisterPetCmd,
    state: Option<PetState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    if state.is_some() {
        return Err(DomainError::Conflict(format!(
            "pet {aggregate_id} already exists"
        )));
    }
    if cmd.name.trim().is_empty() {
        return Err(DomainError::Validation("pet name required".into()));
    }
    let pet_id = aggregate_id.to_string();
    let ev = PetEventDto::Registered(PetRegisteredDto {
        pet_id: pet_id.clone(),
        owner_id: cmd.owner_id,
        name: cmd.name,
        birth_date: cmd.birth_date,
        pet_type_id: cmd.pet_type.id,
        pet_type_name: cmd.pet_type.name,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event("pet", &pet_id, 1, "PetRegistered", &ev)?])
}

fn handle_schedule_visit(
    cmd: ScheduleVisitCmd,
    state: Option<PetState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    let state = state.ok_or_else(|| DomainError::NotFound(cmd.pet_id.clone()))?;
    if cmd.description.trim().is_empty() {
        return Err(DomainError::Validation("visit description required".into()));
    }
    let visit_id = new_id();
    let next_seq = state.version + 1;
    let ev = PetEventDto::VisitScheduled(VisitScheduledDto {
        pet_id: cmd.pet_id.clone(),
        visit_id: visit_id.clone(),
        date: cmd.date,
        description: cmd.description,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event(
        "pet",
        &cmd.pet_id,
        next_seq,
        "VisitScheduled",
        &ev,
    )?])
}

fn apply_pet_event(
    state: Option<PetState>,
    event: &StoredEvent,
) -> Result<PetState, DomainError> {
    let payload = String::from_utf8(event.payload_json.clone())
        .map_err(|e| DomainError::Internal(e.to_string()))?;
    let ev: PetEventDto = serde_json::from_str(&payload)
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match ev {
        PetEventDto::Registered(r) => Ok(PetState {
            pet_id: r.pet_id,
            owner_id: r.owner_id,
            name: r.name,
            birth_date: r.birth_date,
            pet_type: exports::petclinic::pet_aggregate::pet_aggregate::PetTypeRef {
                id: r.pet_type_id,
                name: r.pet_type_name,
            },
            visits: vec![],
            version: event.sequence,
        }),
        PetEventDto::VisitScheduled(v) => {
            let mut s = state.ok_or_else(|| {
                DomainError::Internal("VisitScheduled without prior pet state".into())
            })?;
            s.visits.push(VisitSummary {
                visit_id: v.visit_id,
                date: v.date,
                description: v.description,
            });
            s.version = event.sequence;
            Ok(s)
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
enum PetEventDto {
    Registered(PetRegisteredDto),
    VisitScheduled(VisitScheduledDto),
}

#[derive(Serialize, Deserialize)]
struct PetRegisteredDto {
    pet_id:        String,
    owner_id:      String,
    name:          String,
    birth_date:    String,
    pet_type_id:   String,
    pet_type_name: String,
    occurred_at:   u64,
}

#[derive(Serialize, Deserialize)]
struct VisitScheduledDto {
    pet_id:      String,
    visit_id:    String,
    date:        String,
    description: String,
    occurred_at: u64,
}
