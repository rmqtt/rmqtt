#!/bin/sh

# Trusted server configuration (DNS and IP at least one configuration)
# Priority first DNS then IP.
# Trusted domain name, none can be empty, multiple separated by English spaces.
DNS="rmqtt.com localhost"
# Trusted IP, none can be empty, separate multiples with English spaces.
IP="192.168.1.1 127.0.0.1"

# root ca
ROOT_KEY_FILE="root.key"
ROOT_CERT_FILE="root.pem"
ROOT_CERT_DAYS=3650
ROOT_RSA_KEY_BIT="rsa:4096"

# server ca
SERVER_KEY_FILE="rmqtt.key"
SERVER_CERT_FILE="rmqtt.pem"
SERVER_CERT_FULL_CHAIN_FILE="rmqtt.fullchain.pem"
SERVER_CERT_DAYS=3650
SERVER_REQ_FILE="rmqtt.req"
SERVER_RSA_KEY_BIT="rsa:2048"

# client ca
CLIENT_KEY_FILE="client.key"
CLIENT_CERT_FILE="client.pem"
CLIENT_CERT_DAYS=3650
CLIENT_REQ_FILE="client.req"
CLIENT_RSA_KEY_BIT="rsa:2048"

# temp config file
ROOT_CA_CONFIG_FILE="ca.conf"
ROOT_CA_EXT_FILE="extfile.conf"

gen_ca_conf(){
    if [ -f ${ROOT_CA_CONFIG_FILE} ]; then rm ${ROOT_CA_CONFIG_FILE}; fi
    cat > ${ROOT_CA_CONFIG_FILE} <<EOF
[req]
distinguished_name = req_distinguished_name
prompt = no

[req_distinguished_name]
CN = Server certificate
OU = RMQTT
O = RMQTT
C = CN
EOF
}

gen_ext_file_conf(){
    if [ -f ${ROOT_CA_EXT_FILE} ]; then rm ${ROOT_CA_EXT_FILE}; fi
    cat > ${ROOT_CA_EXT_FILE} <<EOF
[ v3_server ]
basicConstraints = critical, CA:false
keyUsage = nonRepudiation, digitalSignature
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:authorityKeyIdentifier,issuer:always
subjectAltName = @alt_names
extendedKeyUsage = critical, serverAuth

[ v3_client ]
basicConstraints = critical, CA:false
keyUsage = nonRepudiation, digitalSignature
extendedKeyUsage = critical, clientAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:authorityKeyIdentifier,issuer:always

[ alt_names ]
EOF
    index=1
    for d in $DNS;
    do
        echo "DNS.${index} = ${d}" >> ${ROOT_CA_EXT_FILE}
        index=$((index + 1))
    done

    index=1
    for d in $IP;
    do
        echo "IP.${index} = ${d}" >> ${ROOT_CA_EXT_FILE}
        index=$((index + 1))
    done
}

gen_root(){
    if [ ! -f ${ROOT_CA_CONFIG_FILE} ];
    then
        echo "${ROOT_CA_CONFIG_FILE} file not found, please use 'gen.sh conf' to generate";
        exit 1;
    fi

    openssl req -nodes \
        -x509 \
        -days ${ROOT_CERT_DAYS} \
        -newkey ${ROOT_RSA_KEY_BIT} \
        -keyout ${ROOT_KEY_FILE} \
        -out ${ROOT_CERT_FILE} \
        -sha256 \
        -batch \
        -config ${ROOT_CA_CONFIG_FILE}

    sed -i 's/BEGIN PRIVATE KEY/BEGIN RSA PRIVATE KEY/' ${ROOT_KEY_FILE}
    sed -i 's/END PRIVATE KEY/END RSA PRIVATE KEY/' ${ROOT_KEY_FILE}
}

gen_server(){
    if [ ! -f ${ROOT_CA_CONFIG_FILE} ];
    then
        echo "${ROOT_CA_CONFIG_FILE} file not found, please use 'gen.sh conf' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_CA_EXT_FILE} ];
    then
        echo "${ROOT_CA_EXT_FILE} file not found, please use 'gen.sh conf' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_CERT_FILE} ];
    then
        echo "${ROOT_CERT_FILE} file not found, please use 'gen.sh root' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_KEY_FILE} ];
    then
        echo "${ROOT_KEY_FILE} file not found, please use 'gen.sh root' to generate";
        exit 1;
    fi

    openssl req -nodes \
        -newkey ${SERVER_RSA_KEY_BIT} \
        -keyout ${SERVER_KEY_FILE} \
        -out ${SERVER_REQ_FILE} \
        -sha256 \
        -batch \
        -config ${ROOT_CA_CONFIG_FILE}

    openssl x509 -req \
        -in ${SERVER_REQ_FILE} \
        -out ${SERVER_CERT_FILE} \
        -CA ${ROOT_CERT_FILE} \
        -CAkey ${ROOT_KEY_FILE} \
        -sha256 \
        -days ${SERVER_CERT_DAYS} \
        -set_serial 456 \
        -extensions v3_server -extfile ${ROOT_CA_EXT_FILE}

    sed -i 's/BEGIN PRIVATE KEY/BEGIN RSA PRIVATE KEY/' ${SERVER_KEY_FILE}
    sed -i 's/END PRIVATE KEY/END RSA PRIVATE KEY/' ${SERVER_KEY_FILE}

    # fullchain
    cat ${SERVER_CERT_FILE} ${ROOT_CERT_FILE} > ${SERVER_CERT_FULL_CHAIN_FILE}

    openssl verify -verbose -CAfile ${ROOT_CERT_FILE} ${SERVER_CERT_FILE}
    openssl verify -verbose -CAfile ${ROOT_CERT_FILE} ${SERVER_CERT_FULL_CHAIN_FILE}
}

gen_client(){
    if [ ! -f ${ROOT_CA_CONFIG_FILE} ];
    then
        echo "${ROOT_CA_CONFIG_FILE} file not found, please use 'gen.sh conf' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_CA_EXT_FILE} ];
    then
        echo "${ROOT_CA_EXT_FILE} file not found, please use 'gen.sh conf' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_CERT_FILE} ];
    then
        echo "${ROOT_CERT_FILE} file not found, please use 'gen.sh root' to generate";
        exit 1;
    fi

    if [ ! -f ${ROOT_KEY_FILE} ];
    then
        echo "${ROOT_KEY_FILE} file not found, please use 'gen.sh root' to generate";
        exit 1;
    fi

    openssl req -nodes \
        -newkey ${CLIENT_RSA_KEY_BIT} \
        -keyout ${CLIENT_KEY_FILE} \
        -out ${CLIENT_REQ_FILE} \
        -sha256 \
        -batch \
        -config ${ROOT_CA_CONFIG_FILE}

    openssl x509 -req \
        -in ${CLIENT_REQ_FILE} \
        -out ${CLIENT_CERT_FILE} \
        -CA ${ROOT_CERT_FILE} \
        -CAkey ${ROOT_KEY_FILE} \
        -sha256 \
        -days ${CLIENT_CERT_DAYS} \
        -set_serial 789 \
        -extensions v3_client -extfile ${ROOT_CA_EXT_FILE}

    sed -i 's/BEGIN PRIVATE KEY/BEGIN RSA PRIVATE KEY/' ${CLIENT_KEY_FILE}
    sed -i 's/END PRIVATE KEY/END RSA PRIVATE KEY/' ${CLIENT_KEY_FILE}

    openssl verify -verbose -CAfile ${ROOT_CERT_FILE} ${CLIENT_CERT_FILE}
}

case $1 in
    conf)
        echo "generate configuration file..."
        gen_ca_conf
        gen_ext_file_conf
        echo "generate configuration file success"
        ;;
    root)
        echo "generate root cert file..."
        gen_root
        echo "generate root cert file success"
        ;;
    client)
        echo "generate client cert file..."
        gen_client
        echo "generate client cert file success"
        ;;
    server)
        echo "generate server cert file..."
        gen_server
        echo "generate server cert file success"
        ;;
    all)
        echo "generate all..."
        gen_ca_conf
        gen_ext_file_conf
        gen_root
        gen_client
        gen_server
        echo "generate all success"
        ;;
    *)
        echo "Usage gen.sh {conf|root|client|server}" >&2
        exit 1
        ;;
esac
