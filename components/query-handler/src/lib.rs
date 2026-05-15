mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::petclinic::gateway::api::{
    DomainError, Guest, OwnerCommand, OwnerListItem, OwnerProfileView, PetCommand,
    PetSummary, PetTypeRef, SpecialtyRef, VetCommand, VetListItem, VetProfileView, VisitSummary,
};
use bindings::wasi::keyvalue::store;
use serde::{Deserialize, Serialize};

struct Component;

export!(Component);

const VIEWS_BUCKET: &str = "default";

impl Guest for Component {
    fn send_owner_command(_cmd: OwnerCommand) -> Result<String, DomainError> {
        Err(DomainError::Internal(
            "commands routed via api-gateway, not query-handler".into(),
        ))
    }

    fn send_pet_command(_cmd: PetCommand) -> Result<String, DomainError> {
        Err(DomainError::Internal(
            "commands routed via api-gateway, not query-handler".into(),
        ))
    }

    fn send_vet_command(_cmd: VetCommand) -> Result<String, DomainError> {
        Err(DomainError::Internal(
            "commands routed via api-gateway, not query-handler".into(),
        ))
    }

    fn get_owner(owner_id: String) -> Result<OwnerProfileView, DomainError> {
        let bucket = open_bucket()?;
        let key = format!("view.owner.{owner_id}");
        let bytes = bucket
            .get(&key)
            .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            .ok_or_else(|| DomainError::NotFound(owner_id.clone()))?;
        let dto: OwnerProfileViewDto = serde_json::from_slice(&bytes)
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        Ok(dto.into())
    }

    fn list_owners(page: u32, limit: u32) -> Result<Vec<OwnerListItem>, DomainError> {
        let bucket = open_bucket()?;
        let bytes = bucket
            .get("view.owner.list")
            .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            .unwrap_or_default();
        let list: Vec<OwnerListItemDto> = if bytes.is_empty() {
            vec![]
        } else {
            serde_json::from_slice(&bytes).map_err(|e| DomainError::Internal(e.to_string()))?
        };
        let start = ((page.saturating_sub(1)) * limit) as usize;
        Ok(list
            .into_iter()
            .skip(start)
            .take(limit as usize)
            .map(Into::into)
            .collect())
    }

    fn search_owners_by_last_name(last_name: String) -> Result<Vec<OwnerListItem>, DomainError> {
        let bucket = open_bucket()?;
        let idx_key = format!("view.owner.idx.last-name.{}", last_name.to_lowercase());
        let ids: Vec<String> = bucket
            .get(&idx_key)
            .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let mut result = Vec::new();
        for id in ids {
            let key = format!("view.owner.{id}");
            if let Some(bytes) = bucket
                .get(&key)
                .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            {
                let dto: OwnerProfileViewDto = serde_json::from_slice(&bytes)
                    .map_err(|e| DomainError::Internal(e.to_string()))?;
                result.push(OwnerListItem {
                    owner_id:   dto.owner_id,
                    first_name: dto.first_name,
                    last_name:  dto.last_name,
                    city:       dto.city,
                    pet_count:  dto.pets.len() as u32,
                });
            }
        }
        Ok(result)
    }

    fn get_vet(vet_id: String) -> Result<VetProfileView, DomainError> {
        let bucket = open_bucket()?;
        let key = format!("view.vet.{vet_id}");
        let bytes = bucket
            .get(&key)
            .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            .ok_or_else(|| DomainError::NotFound(vet_id.clone()))?;
        let dto: VetProfileViewDto = serde_json::from_slice(&bytes)
            .map_err(|e| DomainError::Internal(e.to_string()))?;
        Ok(dto.into())
    }

    fn list_vets(page: u32, limit: u32) -> Result<Vec<VetListItem>, DomainError> {
        let bucket = open_bucket()?;
        let bytes = bucket
            .get("view.vet.list")
            .map_err(|e| DomainError::StoreError(format!("{e:?}")))?
            .unwrap_or_default();
        let list: Vec<VetListItemDto> = if bytes.is_empty() {
            vec![]
        } else {
            serde_json::from_slice(&bytes).map_err(|e| DomainError::Internal(e.to_string()))?
        };
        let start = ((page.saturating_sub(1)) * limit) as usize;
        Ok(list
            .into_iter()
            .skip(start)
            .take(limit as usize)
            .map(Into::into)
            .collect())
    }
}

fn open_bucket() -> Result<store::Bucket, DomainError> {
    store::open(VIEWS_BUCKET).map_err(|e| DomainError::StoreError(format!("{e:?}")))
}

// ── DTOs and conversions ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct OwnerProfileViewDto {
    owner_id: String, first_name: String, last_name: String,
    address: String, city: String, telephone: String,
    pets: Vec<PetSummaryDto>, version: u64,
}

#[derive(Serialize, Deserialize)]
struct PetSummaryDto {
    pet_id: String, name: String, birth_date: String,
    pet_type_id: String, pet_type_name: String, visits: Vec<VisitSummaryDto>,
}

#[derive(Serialize, Deserialize)]
struct VisitSummaryDto { visit_id: String, date: String, description: String }

#[derive(Serialize, Deserialize)]
struct OwnerListItemDto {
    owner_id: String, first_name: String, last_name: String,
    city: String, pet_count: u32,
}

#[derive(Serialize, Deserialize)]
struct VetProfileViewDto {
    vet_id: String, first_name: String, last_name: String,
    specialties: Vec<SpecialtyDto>, version: u64,
}

#[derive(Serialize, Deserialize)]
struct SpecialtyDto { id: String, name: String }

#[derive(Serialize, Deserialize)]
struct VetListItemDto {
    vet_id: String, first_name: String, last_name: String, specialty_count: u32,
}

impl From<OwnerProfileViewDto> for OwnerProfileView {
    fn from(d: OwnerProfileViewDto) -> Self {
        OwnerProfileView {
            owner_id: d.owner_id, first_name: d.first_name, last_name: d.last_name,
            address: d.address, city: d.city, telephone: d.telephone,
            pets: d.pets.into_iter().map(|p| PetSummary {
                pet_id: p.pet_id, name: p.name, birth_date: p.birth_date,
                pet_type: PetTypeRef { id: p.pet_type_id, name: p.pet_type_name },
                visits: p.visits.into_iter().map(|v| VisitSummary {
                    visit_id: v.visit_id, date: v.date, description: v.description,
                }).collect(),
            }).collect(),
            version: d.version,
        }
    }
}

impl From<OwnerListItemDto> for OwnerListItem {
    fn from(d: OwnerListItemDto) -> Self {
        OwnerListItem { owner_id: d.owner_id, first_name: d.first_name,
            last_name: d.last_name, city: d.city, pet_count: d.pet_count }
    }
}

impl From<VetProfileViewDto> for VetProfileView {
    fn from(d: VetProfileViewDto) -> Self {
        VetProfileView {
            vet_id: d.vet_id, first_name: d.first_name, last_name: d.last_name,
            specialties: d.specialties.into_iter().map(|s| SpecialtyRef { id: s.id, name: s.name }).collect(),
            version: d.version,
        }
    }
}

impl From<VetListItemDto> for VetListItem {
    fn from(d: VetListItemDto) -> Self {
        VetListItem { vet_id: d.vet_id, first_name: d.first_name,
            last_name: d.last_name, specialty_count: d.specialty_count }
    }
}
