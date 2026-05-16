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

        let pet_id = &event.aggregate_id;
        let view_key = format!("view.pet.{pet_id}");

        let mut view: PetProfileViewDto = bucket
            .get(&view_key)
            .map_err(|e| format!("{e:?}"))?
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| PetProfileViewDto {
                pet_id:        pet_id.clone(),
                owner_id:      String::new(),
                name:          String::new(),
                birth_date:    String::new(),
                pet_type_id:   String::new(),
                pet_type_name: String::new(),
                visits:        vec![],
                version:       0,
            });

        let ev: PetEventDto = serde_json::from_str(&event.payload_json)
            .map_err(|e| format!("parse event: {e}"))?;
        match ev {
            PetEventDto::Registered(p) => {
                view.pet_id        = p.pet_id;
                view.owner_id      = p.owner_id;
                view.name          = p.name;
                view.birth_date    = p.birth_date;
                view.pet_type_id   = p.pet_type_id;
                view.pet_type_name = p.pet_type_name;
            }
            PetEventDto::VisitScheduled(v) => {
                view.visits.push(VisitSummaryDto {
                    visit_id:    v.visit_id,
                    date:        v.date,
                    description: v.description,
                });
            }
        }
        view.version = event.sequence;

        let bytes = serde_json::to_vec(&view).map_err(|e| e.to_string())?;
        bucket.set(&view_key, &bytes).map_err(|e| format!("{e:?}"))?;

        update_pet_list(&bucket, &view)?;
        Ok(())
    }
}

fn update_pet_list(
    bucket: &bindings::wasi::keyvalue::store::Bucket,
    view: &PetProfileViewDto,
) -> Result<(), String> {
    let list_key = "view.pet.list";
    let mut list: Vec<PetListItemDto> = bucket
        .get(list_key)
        .map_err(|e| format!("{e:?}"))?
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let item = PetListItemDto {
        pet_id:      view.pet_id.clone(),
        owner_id:    view.owner_id.clone(),
        name:        view.name.clone(),
        visit_count: view.visits.len() as u32,
    };
    if let Some(existing) = list.iter_mut().find(|i| i.pet_id == view.pet_id) {
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
struct PetProfileViewDto {
    pet_id:        String,
    owner_id:      String,
    name:          String,
    birth_date:    String,
    pet_type_id:   String,
    pet_type_name: String,
    visits:        Vec<VisitSummaryDto>,
    version:       u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct VisitSummaryDto {
    visit_id:    String,
    date:        String,
    description: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct PetListItemDto {
    pet_id:      String,
    owner_id:    String,
    name:        String,
    visit_count: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum PetEventDto {
    Registered(PetRegisteredPayload),
    VisitScheduled(VisitScheduledPayload),
}

#[derive(Serialize, Deserialize)]
struct PetRegisteredPayload {
    pet_id: String, owner_id: String, name: String, birth_date: String,
    pet_type_id: String, pet_type_name: String, occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct VisitScheduledPayload {
    pet_id: String, visit_id: String, date: String,
    description: String, occurred_at: u64,
}
