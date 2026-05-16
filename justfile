registry := "localhost:5001"
nats     := "localhost:4222"
lattice  := "petclinic"

components := "api-gateway owner-aggregate pet-aggregate vet-aggregate event-store owner-projector vet-projector pet-projector query-handler ui-server"

# Show available recipes
default:
    @just --list

# ── Build ──────────────────────────────────────────────────────────────────────

# Validate all WIT files (parse + type-check) without compiling Rust
wit-validate:
    #!/usr/bin/env bash
    set -euo pipefail
    for c in {{components}}; do
        echo "→ wit validate: $c"
        (cd components/$c && wash wit fetch)
    done
    echo "✓ all WIT valid"

# Fetch WIT deps for all components
deps:
    #!/usr/bin/env bash
    set -euo pipefail
    for c in {{components}}; do
        echo "→ wit fetch: $c"
        (cd components/$c && wash wit fetch)
    done

# Build all components (skips WIT registry fetch for local petclinic packages)
build:
    #!/usr/bin/env bash
    set -euo pipefail
    for c in {{components}}; do
        echo "→ build: $c"
        (cd components/$c && wash build --skip-fetch)
    done

# Build a single component: just build-one api-gateway
build-one name:
    cd components/{{name}} && wash build --skip-fetch

# ── Kubernetes ─────────────────────────────────────────────────────────────────

# Apply namespace, NATS, registry, WADM, and wasmCloud host (one-time setup)
k8s-up:
    kubectl apply -f deploy/k8s/00-namespace.yaml
    kubectl apply -f deploy/k8s/01-nats.yaml
    kubectl apply -f deploy/k8s/02-registry.yaml
    kubectl apply -f deploy/k8s/03-wadm.yaml
    kubectl apply -f deploy/k8s/04-host.yaml
    kubectl rollout status statefulset/nats     -n petclinic --timeout=120s
    kubectl rollout status deployment/registry  -n petclinic --timeout=60s
    kubectl rollout status deployment/wadm      -n petclinic --timeout=60s

# Destroy everything (deletes the petclinic namespace)
k8s-down:
    kubectl delete namespace petclinic --ignore-not-found

# Port-forward registry to localhost:5001 (run in a separate terminal before k8s-push)
k8s-pf-registry:
    kubectl port-forward -n petclinic svc/registry 5001:5000

# Port-forward NATS to localhost:4222 (run in a separate terminal before k8s-deploy)
k8s-pf-nats:
    kubectl port-forward -n petclinic svc/nats 4222:4222

# Build all components and push images to the in-cluster registry
# Requires: registry port-forward running (just k8s-pf-registry)
k8s-push: build
    #!/usr/bin/env bash
    set -euo pipefail
    for c in {{components}}; do
        name=${c//-/_}
        wasm="target/wasm32-wasip1/release/${name}.wasm"
        echo "→ push: $c → {{registry}}/petclinic/$c:latest"
        wash oci push --insecure {{registry}}/petclinic/$c:latest $wasm
    done

# Push a single component image: just k8s-push-one api-gateway
k8s-push-one name: (build-one name)
    #!/usr/bin/env bash
    set -euo pipefail
    wasm_name=$(echo "{{name}}" | tr '-' '_')
    wash oci push --insecure {{registry}}/petclinic/{{name}}:latest \
        target/wasm32-wasip1/release/${wasm_name}.wasm

# Deploy (or update) the WADM application manifest
# Requires: NATS port-forward running (just k8s-pf-nats)
k8s-deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(grep 'version:' deploy/wadm.yaml | head -1 | awk '{print $2}' | tr -d '"')
    nats --server {{nats}} request "wadm.api.{{lattice}}.model.put" "$(cat deploy/wadm.yaml)"
    nats --server {{nats}} request "wadm.api.{{lattice}}.model.deploy.petclinic" \
        "{\"version\":\"$version\"}"

# Show WADM application status
# Requires: NATS port-forward running (just k8s-pf-nats)
k8s-status:
    nats --server {{nats}} request "wadm.api.{{lattice}}.model.status.petclinic" "" \
        | python3 -m json.tool

# Undeploy and delete the WADM manifest
# Requires: NATS port-forward running (just k8s-pf-nats)
k8s-undeploy:
    nats --server {{nats}} request "wadm.api.{{lattice}}.model.undeploy.petclinic" '{}'
    nats --server {{nats}} request "wadm.api.{{lattice}}.model.delete.petclinic"   '{}'

# ── Smoke test ─────────────────────────────────────────────────────────────────

# Run full smoke test against a live deployment (port-forward api on :8080 first)
test host="http://localhost:8080":
    #!/usr/bin/env bash
    set -euo pipefail

    ts=$(date +%s)

    echo "=== bootstrap admin (first run) or login ==="
    bootstrap=$(curl -s -X POST {{host}}/admin/bootstrap \
      -H 'Content-Type: application/json' \
      -d '{"username":"admin","password":"adminpass123"}')
    echo $bootstrap | python3 -m json.tool
    if echo "$bootstrap" | python3 -c "import sys,json; d=json.load(sys.stdin); exit(0 if 'token' in d else 1)" 2>/dev/null; then
        admin_token=$(echo "$bootstrap" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
    else
        admin_token=$(curl -sf -X POST {{host}}/auth/login \
          -H 'Content-Type: application/json' \
          -d '{"username":"admin","password":"adminpass123"}' \
          | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
    fi
    echo "admin token acquired"

    echo "=== create vet invite ==="
    invite=$(curl -sf -X POST {{host}}/admin/invites \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $admin_token")
    echo $invite | python3 -m json.tool
    invite_token=$(echo $invite | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")

    echo "=== register vet via invite ==="
    vet_username="dr_carter_$ts"
    vet_reg=$(curl -sf -X POST {{host}}/auth/register-vet \
      -H 'Content-Type: application/json' \
      -d "{\"inviteToken\":\"$invite_token\",\"username\":\"$vet_username\",\"password\":\"vetpass123\",\"firstName\":\"James\",\"lastName\":\"Carter\"}")
    echo $vet_reg | python3 -m json.tool
    vet_token=$(echo $vet_reg | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")

    echo "=== register owner (self-register) ==="
    owner_username="owner_franklin_$ts"
    owner_reg=$(curl -sf -X POST {{host}}/auth/register-owner \
      -H 'Content-Type: application/json' \
      -d "{\"username\":\"$owner_username\",\"password\":\"ownerpass123\",\"firstName\":\"George\",\"lastName\":\"Franklin\",\"address\":\"110 W. Liberty St.\",\"city\":\"Madison\",\"telephone\":\"6085551023\"}")
    echo $owner_reg | python3 -m json.tool
    owner_token=$(echo $owner_reg | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
    owner_id=$(echo $owner_reg | python3 -c "import sys,json; print(json.load(sys.stdin)['ownerId'])")

    echo "=== register pet (owner auth) ==="
    pet=$(curl -sf -X POST {{host}}/pets \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $owner_token" \
      -d "{\"name\":\"Leo\",\"birthDate\":\"2020-01-01\",\"petTypeId\":\"cat\",\"petTypeName\":\"Cat\",\"ownerId\":\"$owner_id\"}")
    echo $pet
    pet_id=$(echo $pet | python3 -c "import sys,json; print(json.load(sys.stdin)['petId'])")

    echo "=== upload medical document (vet auth) ==="
    doc=$(curl -sf -X POST {{host}}/pets/$pet_id/medical-documents \
      -H 'Content-Type: application/json' \
      -H "Authorization: Bearer $vet_token" \
      -d '{"title":"Annual Checkup","notes":"All vitals normal. Weight 4.2kg.","attachments":[{"filename":"checkup.txt","contentType":"text/plain","dataBase64":"QWxsIHZpdGFscyBub3JtYWwu"}]}')
    echo $doc | python3 -m json.tool

    echo "=== list medical documents ==="
    curl -sf {{host}}/pets/$pet_id/medical-documents \
      -H "Authorization: Bearer $vet_token" \
      | python3 -m json.tool

    echo "=== list owners (auth required) ==="
    curl -sf {{host}}/owners \
      -H "Authorization: Bearer $vet_token" \
      | python3 -m json.tool

    echo "=== list vets (auth required) ==="
    curl -sf {{host}}/vets \
      -H "Authorization: Bearer $vet_token" \
      | python3 -m json.tool

    echo "=== list invites (admin) ==="
    curl -sf {{host}}/admin/invites \
      -H "Authorization: Bearer $admin_token" \
      | python3 -m json.tool
