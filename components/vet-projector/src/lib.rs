mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::wasmcloud::messaging::handler::{BrokerMessage, Guest};
use bindings::wasi::keyvalue::store;
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

const VIEWS_BUCKET: &str = "default";

impl Guest for Component {
    fn handle_message(msg: BrokerMessage) -> Result<(), String> {
        let event: StoredEventDto = serde_json::from_slice(&msg.body)
            .map_err(|e| format!("deserialise: {e}"))?;

        let bucket = store::open(VIEWS_BUCKET)
            .map_err(|e| format!("open bucket: {e:?}"))?;

        let vet_id = &event.aggregate_id;
        let view_key = format!("view.vet.{vet_id}");

        let mut view: VetProfileViewDto = bucket
            .get(&view_key)
            .map_err(|e| format!("{e:?}"))?
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| VetProfileViewDto {
                vet_id: vet_id.clone(),
                first_name: String::new(),
                last_name: String::new(),
                specialties: vec![],
                version: 0,
            });

        let ev: VetEventDto = serde_json::from_str(&event.payload_json)
            .map_err(|e| format!("parse event: {e}"))?;
        match ev {
            VetEventDto::Registered(p) => {
                view.vet_id = p.vet_id;
                view.first_name = p.first_name;
                view.last_name = p.last_name;
            }
            VetEventDto::SpecialtyAdded(p) => {
                if !view.specialties.iter().any(|s| s.id == p.specialty_id) {
                    view.specialties.push(SpecialtyDto {
                        id: p.specialty_id,
                        name: p.specialty_name,
                    });
                }
            }
        }
        view.version = event.sequence;

        let bytes = serde_json::to_vec(&view).map_err(|e| e.to_string())?;
        bucket.set(&view_key, &bytes).map_err(|e| format!("{e:?}"))?;

        update_vet_list(&bucket, &view)?;
        Ok(())
    }
}

fn update_vet_list(
    bucket: &bindings::wasi::keyvalue::store::Bucket,
    view: &VetProfileViewDto,
) -> Result<(), String> {
    let list_key = "view.vet.list";
    let mut list: Vec<VetListItemDto> = bucket
        .get(list_key)
        .map_err(|e| format!("{e:?}"))?
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let item = VetListItemDto {
        vet_id:          view.vet_id.clone(),
        first_name:      view.first_name.clone(),
        last_name:       view.last_name.clone(),
        specialty_count: view.specialties.len() as u32,
    };
    if let Some(existing) = list.iter_mut().find(|i| i.vet_id == view.vet_id) {
        *existing = item;
    } else {
        list.push(item);
    }
    let bytes = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
    bucket.set(list_key, &bytes).map_err(|e| format!("{e:?}"))
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct StoredEventDto {
    event_id: String, aggregate_type: String, aggregate_id: String,
    sequence: u64, event_type: String, payload_json: String, occurred_at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct VetProfileViewDto {
    vet_id:      String,
    first_name:  String,
    last_name:   String,
    specialties: Vec<SpecialtyDto>,
    version:     u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct SpecialtyDto { id: String, name: String }

#[derive(Serialize, Deserialize, Clone)]
struct VetListItemDto {
    vet_id: String, first_name: String, last_name: String, specialty_count: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum VetEventDto {
    Registered(VetRegisteredPayload),
    SpecialtyAdded(SpecialtyAddedPayload),
}

#[derive(Serialize, Deserialize)]
struct VetRegisteredPayload {
    vet_id: String, first_name: String, last_name: String, occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct SpecialtyAddedPayload {
    vet_id: String, specialty_id: String, specialty_name: String, occurred_at: u64,
}
