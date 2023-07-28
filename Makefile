all: release docker

debug:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --target x86_64-unknown-linux-musl

release:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --release --target x86_64-unknown-linux-musl

docker:
	podman build --no-cache -t rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1)) ./
	podman build --no-cache -t rmqtt/rmqtt:latest ./

publish:
	podman push rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))
	podman push rmqtt/rmqtt:latest

clean:
	cargo clean
