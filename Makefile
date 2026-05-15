COMPONENTS := api-gateway owner-aggregate pet-aggregate vet-aggregate \
              event-store owner-projector vet-projector query-handler ui-server

.PHONY: all deps build sign up down deploy undeploy test clean help

## Fetch WASI/wasmCloud WIT deps for every component
deps:
	@for c in $(COMPONENTS); do \
	  echo "→ wit fetch: $$c"; \
	  (cd components/$$c && wash wit fetch) || exit 1; \
	done

## Compile all components to wasm32-wasip2
build:
	@for c in $(COMPONENTS); do \
	  echo "→ build: $$c"; \
	  (cd components/$$c && wash build) || exit 1; \
	done

## Full bootstrap: fetch deps then build
all: deps build

## Start local wasmCloud host + NATS (background)
up:
	wash up --detached

## Stop wasmCloud host
down:
	wash down

## Deploy application via WADM
deploy:
	wash app deploy deploy/wadm.yaml

## Remove deployed application
undeploy:
	wash app delete petclinic

## Show running app status
status:
	wash app list

## Smoke-test round-trip (requires running deployment on :8080)
test:
	@echo "--- register owner ---"
	curl -sf -X POST http://localhost:8080/owners \
	  -H 'Content-Type: application/json' \
	  -d '{"firstName":"George","lastName":"Franklin","address":"110 W. Liberty St.","city":"Madison","telephone":"6085551023"}' \
	  | tee /tmp/pc_owner.json && echo
	@echo "--- register vet ---"
	curl -sf -X POST http://localhost:8080/vets \
	  -H 'Content-Type: application/json' \
	  -d '{"firstName":"James","lastName":"Carter"}' \
	  | tee /tmp/pc_vet.json && echo
	@echo "--- list owners ---"
	curl -sf http://localhost:8080/owners | python3 -m json.tool
	@echo "--- list vets ---"
	curl -sf http://localhost:8080/vets | python3 -m json.tool
	@OWNER_ID=$$(python3 -c "import json; print(json.load(open('/tmp/pc_owner.json'))['ownerId'])"); \
	  echo "--- get owner $$OWNER_ID ---"; \
	  curl -sf http://localhost:8080/owners/$$OWNER_ID | python3 -m json.tool

## Remove compiled artifacts
clean:
	cargo clean
	rm -rf build/

help:
	@grep -E '^##' Makefile | sed 's/## //'
