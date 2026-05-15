mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::wasmcloud::messaging::handler::{Guest, BrokerMessage};
use bindings::wasi::keyvalue::store::{self, Bucket};
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

const VIEWS_BUCKET: &str = "default";

impl Guest for Component {
    fn handle_message(msg: BrokerMessage) -> Result<(), String> {
        let event: StoredEventDto = serde_json::from_slice(&msg.body)
            .map_err(|e| format!("deserialise event: {e}"))?;

        let bucket = store::open(VIEWS_BUCKET)
            .map_err(|e| format!("open views bucket: {e:?}"))?;

        let owner_id = &event.aggregate_id;
        let view_key = format!("view.owner.{owner_id}");

        let mut view: OwnerProfileViewDto = bucket
            .get(&view_key)
            .map_err(|e| format!("read view: {e:?}"))?
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| OwnerProfileViewDto {
                owner_id: owner_id.clone(),
                first_name: String::new(),
                last_name: String::new(),
                address: String::new(),
                city: String::new(),
                telephone: String::new(),
                pets: vec![],
                version: 0,
            });

        apply_event_to_view(&mut view, &event)?;
        view.version = event.sequence;

        let serialised = serde_json::to_vec(&view)
            .map_err(|e| format!("serialise view: {e}"))?;
        bucket
            .set(&view_key, &serialised)
            .map_err(|e| format!("write view: {e:?}"))?;

        update_list_index(&bucket, &view)?;
        update_last_name_index(&bucket, &view)?;

        Ok(())
    }
}

fn apply_event_to_view(
    view: &mut OwnerProfileViewDto,
    event: &StoredEventDto,
) -> Result<(), String> {
    let ev: OwnerEventDto = serde_json::from_str(&event.payload_json)
        .map_err(|e| format!("parse event payload: {e}"))?;
    match ev {
        OwnerEventDto::Registered(r) => {
            view.owner_id   = r.owner_id;
            view.first_name = r.first_name;
            view.last_name  = r.last_name;
            view.address    = r.address;
            view.city       = r.city;
            view.telephone  = r.telephone;
        }
        OwnerEventDto::Updated(u) => {
            if let Some(v) = u.first_name { view.first_name = v; }
            if let Some(v) = u.last_name  { view.last_name  = v; }
            if let Some(v) = u.address    { view.address    = v; }
            if let Some(v) = u.city       { view.city       = v; }
            if let Some(v) = u.telephone  { view.telephone  = v; }
        }
        OwnerEventDto::PetAdded(p) => {
            view.pets.push(PetSummaryDto {
                pet_id:        p.pet_id,
                name:          p.name,
                birth_date:    p.birth_date,
                pet_type_id:   p.pet_type_id,
                pet_type_name: p.pet_type_name,
                visits:        vec![],
            });
        }
        OwnerEventDto::VisitScheduled(v) => {
            if let Some(pet) = view.pets.iter_mut().find(|p| p.pet_id == v.pet_id) {
                pet.visits.push(VisitSummaryDto {
                    visit_id:    v.visit_id,
                    date:        v.date,
                    description: v.description,
                });
            }
        }
    }
    Ok(())
}

fn update_list_index(bucket: &Bucket, view: &OwnerProfileViewDto) -> Result<(), String> {
    let list_key = "view.owner.list";
    let mut list: Vec<OwnerListItemDto> = bucket
        .get(list_key)
        .map_err(|e| format!("{e:?}"))?
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let item = OwnerListItemDto {
        owner_id:   view.owner_id.clone(),
        first_name: view.first_name.clone(),
        last_name:  view.last_name.clone(),
        city:       view.city.clone(),
        pet_count:  view.pets.len() as u32,
    };
    if let Some(existing) = list.iter_mut().find(|i| i.owner_id == view.owner_id) {
        *existing = item;
    } else {
        list.push(item);
    }
    let bytes = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
    bucket.set(list_key, &bytes).map_err(|e| format!("{e:?}"))
}

fn update_last_name_index(bucket: &Bucket, view: &OwnerProfileViewDto) -> Result<(), String> {
    let idx_key = format!("view.owner.idx.last-name.{}", view.last_name.to_lowercase());
    let mut ids: Vec<String> = bucket
        .get(&idx_key)
        .map_err(|e| format!("{e:?}"))?
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if !ids.contains(&view.owner_id) {
        ids.push(view.owner_id.clone());
        let bytes = serde_json::to_vec(&ids).map_err(|e| e.to_string())?;
        bucket.set(&idx_key, &bytes).map_err(|e| format!("{e:?}"))?;
    }
    Ok(())
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct StoredEventDto {
    event_id:       String,
    aggregate_type: String,
    aggregate_id:   String,
    sequence:       u64,
    event_type:     String,
    payload_json:   String,
    occurred_at_ms: u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct OwnerProfileViewDto {
    owner_id:   String,
    first_name: String,
    last_name:  String,
    address:    String,
    city:       String,
    telephone:  String,
    pets:       Vec<PetSummaryDto>,
    version:    u64,
}

#[derive(Serialize, Deserialize, Clone)]
struct PetSummaryDto {
    pet_id:        String,
    name:          String,
    birth_date:    String,
    pet_type_id:   String,
    pet_type_name: String,
    visits:        Vec<VisitSummaryDto>,
}

#[derive(Serialize, Deserialize, Clone)]
struct VisitSummaryDto {
    visit_id:    String,
    date:        String,
    description: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct OwnerListItemDto {
    owner_id:   String,
    first_name: String,
    last_name:  String,
    city:       String,
    pet_count:  u32,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum OwnerEventDto {
    Registered(OwnerRegisteredPayload),
    Updated(OwnerUpdatedPayload),
    PetAdded(PetAddedPayload),
    VisitScheduled(VisitScheduledPayload),
}

#[derive(Serialize, Deserialize)]
struct OwnerRegisteredPayload {
    owner_id: String, first_name: String, last_name: String,
    address: String, city: String, telephone: String, occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct OwnerUpdatedPayload {
    owner_id:   String,
    first_name: Option<String>, last_name: Option<String>,
    address:    Option<String>, city:      Option<String>,
    telephone:  Option<String>, occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct PetAddedPayload {
    owner_id: String, pet_id: String, name: String, birth_date: String,
    pet_type_id: String, pet_type_name: String, occurred_at: u64,
}

#[derive(Serialize, Deserialize)]
struct VisitScheduledPayload {
    pet_id: String, visit_id: String, date: String,
    description: String, occurred_at: u64,
}
