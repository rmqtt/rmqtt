FROM alpine:3.16.2
LABEL maintainer="rmqtt <rmqttd@126.com>"

RUN mkdir -p /app/rmqtt/rmqtt-bin
RUN mkdir -p /app/rmqtt/rmqtt-plugins
COPY target/x86_64-unknown-linux-musl/release/rmqttd /app/rmqtt/rmqtt-bin/
COPY rmqtt.toml /app/rmqtt/
COPY rmqtt-plugins/*.toml /app/rmqtt/rmqtt-plugins/
COPY rmqtt-bin/rmqtt.pem  /app/rmqtt/rmqtt-bin/
COPY rmqtt-bin/rmqtt.key  /app/rmqtt/rmqtt-bin/

WORKDIR /app/rmqtt

VOLUME ["/var/log/rmqtt"]

# rmqtt will occupy these port:
# - 1883 port for MQTT
# - 8883 port for MQTT(TLS)
# - 11883 port for internal MQTT/TCP
# - 6060 for APIs
# - 6003 default raft port
# - 5363 for rpc
EXPOSE 1883 8883 11883 6060 6003 5363

ENTRYPOINT ["/app/rmqtt/rmqtt-bin/rmqttd"]
