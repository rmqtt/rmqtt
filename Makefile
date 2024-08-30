all: release-docker

release-docker:
	podman build --no-cache -t rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1)) ./
	podman build --no-cache -t rmqtt/rmqtt:latest ./

release-amd64:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --release --target x86_64-unknown-linux-musl

docker-amd64:
	podman buildx build --platform linux/amd64 -t rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-amd64 -f Dockerfile.amd64 --load ./
	podman buildx build --platform linux/amd64 -t rmqtt/rmqtt:latest-amd64 -f Dockerfile.amd64 --load ./

publish-amd64:
	podman push rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-amd64
	podman push rmqtt/rmqtt:latest-amd64

release-aarch64:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --release --target aarch64-unknown-linux-musl

docker-aarch64:
	podman buildx build --platform linux/arm64 -t rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-arm64 -f Dockerfile.aarch64 --load ./
	podman buildx build --platform linux/arm64 -t rmqtt/rmqtt:latest-arm64 -f Dockerfile.aarch64 --load ./

publish-arm64:
	podman push rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-arm64
	podman push rmqtt/rmqtt:latest-arm64

merge:
	docker buildx imagetools create --tag rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1)) rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-amd64 rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))-arm64
	docker buildx imagetools create --tag rmqtt/rmqtt:latest rmqtt/rmqtt:latest-amd64 rmqtt/rmqtt:latest-arm64

clean:
	cargo clean
