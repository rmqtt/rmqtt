import { Spin, Table } from "antd";
import React, { useEffect, useState } from "react"
import { axionsInstance } from "../utils/constants";

const Clients: React.FC = ()=> {
    const [isLoading, setIsLoading] = useState<boolean>(false)
    const [dataSource, setDataSource] = useState<Object[]>([])


    useEffect(()=> {

        const fetchBokers = async () => {

            axionsInstance.get("clients").then(response => {
                setDataSource(response.data)
                setIsLoading(true)
            }).catch(error=> {
                console.log(error)
            })

        }

        const interval = setInterval(fetchBokers, 2000)

        return ( )=> {
            clearInterval(interval)
        }
    
    }
    )


    const columns = [
        {
          title: 'Node id',
          dataIndex: 'node_id',
          key: 'node_id',
        },
        {
          title: 'Client Id',
          dataIndex: 'clientid',
          key: 'clientid',
        },
        {
          title: 'User name',
          dataIndex: 'username',
          key: 'username',
        },
        {
            title: 'Protocol Version',
            dataIndex: 'proto_ver',
            key: 'proto_ver',
        },

        {
            title: 'Ip Adresse',
            dataIndex: 'ip_address',
            key: 'ip_address',
        },

        {
            title: 'Client Port',
            dataIndex: 'port',
            key: 'port',
        },
        {
            title: 'connected_at',
            dataIndex: 'connected_at',
            key: 'connected_at',
        },
        {
            title: 'disconnected_at',
            dataIndex: 'disconnected_at',
            key: 'disconnected_at',
        },
        {
            title: 'disconnected_reason',
            dataIndex: 'disconnected_reason',
            key: 'disconnected_reason',
        },
        {
            title: 'connected',
            dataIndex: 'connected',
            key: 'connected',
        },
        {
            title: 'keepalive',
            dataIndex: 'keepalive',
            key: 'keepalive',
        },
        {
            title: 'clean_start',
            dataIndex: 'clean_start',
            key: 'clean_start',
        },
        {
            title: 'expiry_interval',
            dataIndex: 'expiry_interval',
            key: 'expiry_interval',
        },
        {
            title: 'subscriptions_cnt',
            dataIndex: 'subscriptions_cnt',
            key: 'subscriptions_cnt',
        },
        {
            title: 'max_subscriptions',
            dataIndex: 'max_subscriptions',
            key: 'max_subscriptions',
        },
        {
            title: 'inflight',
            dataIndex: 'inflight',
            key: 'inflight',
        },
        {
            title: 'max_inflight',
            dataIndex: 'max_inflight',
            key: 'max_inflight',
        },
        {
            title: 'mqueue_len',
            dataIndex: 'mqueue_len',
            key: 'mqueue_len',
        },
        {
            title: 'max_mqueue',
            dataIndex: 'max_mqueue',
            key: 'max_mqueue',
        },
        {
            title: 'extra_attrs',
            dataIndex: 'extra_attrs',
            key: 'extra_attrs',
        },
      ];

    return  <>
                {isLoading ? (
                    <Table dataSource={dataSource} columns={columns} />
                    
                    ): (<Spin tip="Loading..." size="large">
                            <div className="content" />
                        </Spin>
                        )
                }
            </>
}

export default Clients