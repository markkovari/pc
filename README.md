# PetClinic — wasmCloud 2.x

Event-sourced, CQRS, WIT-first veterinary clinic app running on wasmCloud.

## Architecture

```
HTTP :8080
  └─ api-gateway          (auth, routing, medical docs)
       ├─ owner-aggregate  (write: owner commands → events)
       ├─ pet-aggregate    (write: pet commands → events)
       ├─ vet-aggregate    (write: vet commands → events)
       ├─ event-store      (NATS KV: petclinic-events bucket)
       └─ query-handler    (read: projections from petclinic-views bucket)
            ├─ owner-projector  (owner events → view)
            └─ vet-projector    (vet events → view)

Capability providers:
  httpserver      — wasi:http  (0.0.0.0:8080)
  keyvalue-nats   — wasi:keyvalue  (NATS JetStream KV)
  messaging-nats  — wasmcloud:messaging  (event fan-out)
```

## Features

- **Owners / Pets / Vets** — CRUD via CQRS + event sourcing
- **Auth** — register + JWT login (HS256, pure Rust, WASM-compatible)
- **Medical documents** — vets upload text/image records per pet (stored in KV)

## Prerequisites

- [Rust](https://rustup.rs) + `wasm32-wasip1` target: `rustup target add wasm32-wasip1`
- [wash](https://wasmcloud.com/docs/installation) 2.x
- [just](https://github.com/casey/just)
- [nats CLI](https://github.com/nats-io/natscli)
- `kubectl` with cluster access (for Kubernetes deploy)

## Build

```sh
# Build all components (skips WIT registry fetch — petclinic WIT is local)
just build

# Build a single component
just build-one api-gateway
```

## Kubernetes deployment

### 1. Cluster setup (one-time)

Requires the [wasmcloud-operator](https://github.com/wasmCloud/wasmcloud-operator) installed cluster-wide.

```sh
just k8s-up
```

This creates the `petclinic` namespace and starts:
- NATS (JetStream enabled, 10 Gi PVC)
- OCI registry
- WADM (application orchestrator)
- wasmCloud host (lattice: `petclinic`)

### 2. Push component images

Open a terminal and keep the registry port-forward running:

```sh
just k8s-pf-registry   # localhost:5001 → registry.petclinic.svc.cluster.local:5000
```

In another terminal:

```sh
just k8s-push          # builds + pushes all 9 components
```

### 3. Deploy the app

Open a terminal and keep the NATS port-forward running:

```sh
just k8s-pf-nats       # localhost:4222 → nats.petclinic.svc.cluster.local:4222
```

In another terminal:

```sh
just k8s-deploy        # puts wadm.yaml + triggers deploy
just k8s-status        # watch reconciliation
```

### 4. Access the API

```sh
kubectl port-forward -n petclinic svc/petclinic-host 8080:8080
```

### 5. Tear down

```sh
just k8s-down          # deletes the petclinic namespace (all resources)
```

## API reference

### Auth

```sh
# Register
curl -X POST http://localhost:8080/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"username":"dr_carter","password":"secret","role":"vet","entityId":"<vet-id>"}'

# Login → returns JWT token
curl -X POST http://localhost:8080/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"dr_carter","password":"secret"}'
```

### Medical documents

```sh
# Upload (requires Authorization: Bearer <token>)
curl -X POST http://localhost:8080/pets/<pet-id>/medical-documents \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer <token>' \
  -d '{"title":"Annual Checkup","content":"All vitals normal.","mimeType":"text/plain"}'

# List
curl http://localhost:8080/pets/<pet-id>/medical-documents \
  -H 'Authorization: Bearer <token>'
```

### Owners / Pets / Vets

```sh
curl -X POST http://localhost:8080/owners    -H 'Content-Type: application/json' \
  -d '{"firstName":"George","lastName":"Franklin","address":"110 W. Liberty St.","city":"Madison","telephone":"6085551023"}'

curl http://localhost:8080/owners
curl http://localhost:8080/owners/<id>

curl -X POST http://localhost:8080/vets      -H 'Content-Type: application/json' \
  -d '{"firstName":"James","lastName":"Carter"}'

curl http://localhost:8080/vets

curl -X POST http://localhost:8080/pets      -H 'Content-Type: application/json' \
  -d '{"name":"Leo","birthDate":"2020-01-01","species":"cat","ownerId":"<owner-id>"}'
```

## Smoke test

```sh
just test   # runs full round-trip: vet register → login → pet → upload doc → list
```

## KV storage layout

| Key | Content |
|-----|---------|
| `user.<username>` | `UserRecord` (hashed password, role, entity-id) |
| `medoc.<doc-id>` | `MedicalDocRecord` (metadata) |
| `medoc.idx.pet.<pet-id>` | JSON array of doc-ids for a pet |
| `medoc.blob.<doc-id>.<chunk>` | Raw content chunks (base64 for binary) |
