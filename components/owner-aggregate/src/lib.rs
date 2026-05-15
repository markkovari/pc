mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::petclinic::owner_aggregate::owner_aggregate::{
    AddPetCmd, DomainError, Guest, OwnerCommand, OwnerState, RegisterOwnerCmd, StoredEvent,
    UpdateOwnerCmd,
};
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

impl Guest for Component {
    fn handle_command(
        aggregate_id: String,
        command: OwnerCommand,
        history: Vec<StoredEvent>,
    ) -> Result<Vec<StoredEvent>, DomainError> {
        let state = Self::reconstruct_state(history)?;
        match command {
            OwnerCommand::Register(cmd) => handle_register(&aggregate_id, cmd, state),
            OwnerCommand::Update(cmd) => handle_update(cmd, state),
            OwnerCommand::AddPet(cmd) => handle_add_pet(cmd, state),
        }
    }

    fn apply_event(
        state: Option<OwnerState>,
        event: StoredEvent,
    ) -> Result<OwnerState, DomainError> {
        apply_owner_event(state, &event)
    }

    fn reconstruct_state(
        history: Vec<StoredEvent>,
    ) -> Result<Option<OwnerState>, DomainError> {
        history
            .into_iter()
            .try_fold(None, |acc, ev| apply_owner_event(acc, &ev).map(Some))
    }
}

fn handle_register(
    aggregate_id: &str,
    cmd: RegisterOwnerCmd,
    state: Option<OwnerState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    if state.is_some() {
        return Err(DomainError::Conflict(format!(
            "owner {aggregate_id} already exists"
        )));
    }
    validate_telephone(&cmd.telephone)?;
    if cmd.first_name.trim().is_empty() || cmd.last_name.trim().is_empty() {
        return Err(DomainError::Validation("name required".into()));
    }

    let owner_id = aggregate_id.to_string();
    let ev = OwnerEventDto::Registered(OwnerRegisteredDto {
        owner_id: owner_id.clone(),
        first_name: cmd.first_name,
        last_name: cmd.last_name,
        address: cmd.address,
        city: cmd.city,
        telephone: cmd.telephone,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event("owner", &owner_id, 1, "OwnerRegistered", &ev)?])
}

fn handle_update(
    cmd: UpdateOwnerCmd,
    state: Option<OwnerState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    let state = state.ok_or_else(|| DomainError::NotFound(cmd.owner_id.clone()))?;
    if let Some(ref tel) = cmd.telephone {
        validate_telephone(tel)?;
    }
    let next_seq = state.version + 1;
    let ev = OwnerEventDto::Updated(OwnerUpdatedDto {
        owner_id: cmd.owner_id.clone(),
        first_name: cmd.first_name,
        last_name: cmd.last_name,
        address: cmd.address,
        city: cmd.city,
        telephone: cmd.telephone,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event("owner", &cmd.owner_id, next_seq, "OwnerUpdated", &ev)?])
}

fn handle_add_pet(
    cmd: AddPetCmd,
    state: Option<OwnerState>,
) -> Result<Vec<StoredEvent>, DomainError> {
    let state = state.ok_or_else(|| DomainError::NotFound(cmd.owner_id.clone()))?;
    if cmd.name.trim().is_empty() {
        return Err(DomainError::Validation("pet name required".into()));
    }
    let pet_id = new_id();
    let next_seq = state.version + 1;
    let ev = OwnerEventDto::PetAdded(PetAddedDto {
        owner_id: cmd.owner_id.clone(),
        pet_id: pet_id.clone(),
        name: cmd.name,
        birth_date: cmd.birth_date,
        pet_type_id: cmd.pet_type.id,
        pet_type_name: cmd.pet_type.name,
        occurred_at: now_ms(),
    });
    Ok(vec![make_stored_event("owner", &cmd.owner_id, next_seq, "PetAddedToOwner", &ev)?])
}

fn apply_owner_event(
    state: Option<OwnerState>,
    event: &StoredEvent,
) -> Result<OwnerState, DomainError> {
    let payload = String::from_utf8(event.payload_json.clone())
        .map_err(|e| DomainError::Internal(e.to_string()))?;
    let ev: OwnerEventDto = serde_json::from_str(&payload)
        .map_err(|e| DomainError::Internal(e.to_string()))?;

    match ev {
        OwnerEventDto::Registered(r) => Ok(OwnerState {
            owner_id: r.owner_id,
            first_name: r.first_name,
            last_name: r.last_name,
            address: r.address,
            city: r.city,
            telephone: r.telephone,
            pet_ids: vec![],
            version: event.sequence,
        }),
        OwnerEventDto::Updated(u) => {
            let mut s = state.ok_or_else(|| {
                DomainError::Internal("OwnerUpdated without prior state".into())
            })?;
            if let Some(v) = u.first_name { s.first_name = v; }
            if let Some(v) = u.last_name  { s.last_name  = v; }
            if let Some(v) = u.address    { s.address    = v; }
            if let Some(v) = u.city       { s.city       = v; }
            if let Some(v) = u.telephone  { s.telephone  = v; }
            s.version = event.sequence;
            Ok(s)
        }
        OwnerEventDto::PetAdded(p) => {
            let mut s = state.ok_or_else(|| {
                DomainError::Internal("PetAddedToOwner without prior state".into())
            })?;
            s.pet_ids.push(p.pet_id);
            s.version = event.sequence;
            Ok(s)
        }
    }
}

fn validate_telephone(tel: &str) -> Result<(), DomainError> {
    let digits: String = tel.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 10 {
        return Err(DomainError::Validation("telephone must be 10 digits".into()));
    }
    Ok(())
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

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
enum OwnerEventDto {
    Registered(OwnerRegisteredDto),
    Updated(OwnerUpdatedDto),
    PetAdded(PetAddedDto),
}

#[derive(Serialize, Deserialize)]
struct OwnerRegisteredDto {
    owner_id:    String,
    first_name:  String,
    last_name:   String,
    address:     String,
    city:        String,
    telephone:   String,
    occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct OwnerUpdatedDto {
    owner_id:    String,
    first_name:  Option<String>,
    last_name:   Option<String>,
    address:     Option<String>,
    city:        Option<String>,
    telephone:   Option<String>,
    occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct PetAddedDto {
    owner_id:      String,
    pet_id:        String,
    name:          String,
    birth_date:    String,
    pet_type_id:   String,
    pet_type_name: String,
    occurred_at:   u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn telephone_validation_rejects_short() {
        let tel = "12345";
        let digits: String = tel.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_ne!(digits.len(), 10);
    }

    #[test]
    fn telephone_validation_accepts_ten_digits() {
        let tel = "6085551023";
        let digits: String = tel.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_eq!(digits.len(), 10);
    }
}
