mod bindings;
pub use bindings::exports;
use bindings::export;

use exports::petclinic::store::event_store::{Guest, StoredEvent, StoreError};
use bindings::wasi::keyvalue::store::{self, Bucket};
use bindings::wasmcloud::messaging::consumer::{self, BrokerMessage};
use serde_json;

struct Component;

export!(Component);

const EVENTS_BUCKET: &str = "default";
const NATS_TOPIC_PREFIX: &str = "petclinic.events";

impl Guest for Component {
    fn append_events(
        aggregate_type: String,
        aggregate_id: String,
        expected_sequence: u64,
        events: Vec<StoredEvent>,
    ) -> Result<u64, StoreError> {
        let bucket = open_bucket(EVENTS_BUCKET)?;
        let seq_key = seq_key(&aggregate_type, &aggregate_id);

        // Read current sequence
        let current_seq = read_sequence(&bucket, &seq_key)?;
        if current_seq != expected_sequence {
            return Err(StoreError::Conflict(format!(
                "expected sequence {expected_sequence}, got {current_seq}"
            )));
        }

        let mut last_seq = current_seq;
        for event in &events {
            last_seq += 1;
            let key = evt_key(&aggregate_type, &aggregate_id, last_seq);
            let payload = serde_json::to_vec(&StoredEventDto::from(event))
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            bucket
                .set(&key, &payload)
                .map_err(|e| StoreError::Backend(format!("{e:?}")))?;
        }

        // Update sequence counter
        write_sequence(&bucket, &seq_key, last_seq)?;

        // Publish each event to NATS
        for event in &events {
            let topic = format!("{NATS_TOPIC_PREFIX}.{}.{}", aggregate_type, event.event_type);
            let body = serde_json::to_vec(&StoredEventDto::from(event))
                .unwrap_or_default();
            let msg = BrokerMessage {
                subject: topic,
                body,
                reply_to: None,
            };
            consumer::publish(&msg)
                .map_err(|e| StoreError::Backend(format!("publish: {e:?}")))?;
        }

        Ok(last_seq)
    }

    fn load_events(
        aggregate_type: String,
        aggregate_id: String,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        load_from(&aggregate_type, &aggregate_id, 0)
    }

    fn load_events_from(
        aggregate_type: String,
        aggregate_id: String,
        after_sequence: u64,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        load_from(&aggregate_type, &aggregate_id, after_sequence)
    }
}

fn load_from(
    aggregate_type: &str,
    aggregate_id: &str,
    after_sequence: u64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let bucket = open_bucket(EVENTS_BUCKET)?;
    let seq_key = seq_key(aggregate_type, aggregate_id);
    let current_seq = read_sequence(&bucket, &seq_key)?;

    let mut events = Vec::new();
    for seq in (after_sequence + 1)..=current_seq {
        let key = evt_key(aggregate_type, aggregate_id, seq);
        match bucket.get(&key).map_err(|e| StoreError::Backend(format!("{e:?}")))? {
            Some(bytes) => {
                let dto: StoredEventDto = serde_json::from_slice(&bytes)
                    .map_err(|e| StoreError::Backend(e.to_string()))?;
                events.push(StoredEvent::from(dto));
            }
            None => break,
        }
    }
    Ok(events)
}

fn open_bucket(name: &str) -> Result<Bucket, StoreError> {
    store::open(name).map_err(|e| StoreError::Backend(format!("{e:?}")))
}

fn seq_key(aggregate_type: &str, aggregate_id: &str) -> String {
    format!("seq.{aggregate_type}.{aggregate_id}")
}

fn evt_key(aggregate_type: &str, aggregate_id: &str, seq: u64) -> String {
    format!("evt.{aggregate_type}.{aggregate_id}.{seq:020}")
}

fn read_sequence(bucket: &Bucket, key: &str) -> Result<u64, StoreError> {
    match bucket.get(key).map_err(|e| StoreError::Backend(format!("{e:?}")))? {
        Some(bytes) => {
            let s = String::from_utf8(bytes)
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            s.trim().parse::<u64>()
                .map_err(|e| StoreError::Backend(e.to_string()))
        }
        None => Ok(0),
    }
}

fn write_sequence(bucket: &Bucket, key: &str, seq: u64) -> Result<(), StoreError> {
    bucket
        .set(key, seq.to_string().as_bytes())
        .map_err(|e| StoreError::Backend(format!("{e:?}")))
}

// ── DTO for JSON serialisation of StoredEvent ────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredEventDto {
    event_id:       String,
    aggregate_type: String,
    aggregate_id:   String,
    sequence:       u64,
    event_type:     String,
    payload_json:   String,
    occurred_at_ms: u64,
}

impl From<&StoredEvent> for StoredEventDto {
    fn from(e: &StoredEvent) -> Self {
        Self {
            event_id:       e.event_id.clone(),
            aggregate_type: e.aggregate_type.clone(),
            aggregate_id:   e.aggregate_id.clone(),
            sequence:       e.sequence,
            event_type:     e.event_type.clone(),
            payload_json:   String::from_utf8_lossy(&e.payload_json).into_owned(),
            occurred_at_ms: e.occurred_at_ms,
        }
    }
}

impl From<StoredEventDto> for StoredEvent {
    fn from(d: StoredEventDto) -> Self {
        Self {
            event_id:       d.event_id,
            aggregate_type: d.aggregate_type,
            aggregate_id:   d.aggregate_id,
            sequence:       d.sequence,
            event_type:     d.event_type,
            payload_json:   d.payload_json.into_bytes(),
            occurred_at_ms: d.occurred_at_ms,
        }
    }
}
