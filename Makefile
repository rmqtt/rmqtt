all: release docker

debug:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --target x86_64-unknown-linux-musl

release:
	git checkout $$(git describe --tags $$(git rev-list --tags --max-count=1))
	cargo build --release --target x86_64-unknown-linux-musl

docker:
	docker build --no-cache -t rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1)) ./

publish:
	docker push rmqtt/rmqtt:$$(git describe --tags $$(git rev-list --tags --max-count=1))

clean:
	cargo clean
