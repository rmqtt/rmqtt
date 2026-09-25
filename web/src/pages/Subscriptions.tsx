import { Spin, Table } from "antd";
import React, { useEffect, useState } from "react"
import { axionsInstance } from "../utils/constants";

const Subscriptions: React.FC = ()=> {
    const [isLoading, setIsLoading] = useState<boolean>(false)
    const [dataSource, setDataSource] = useState<Object[]>([])


    useEffect(()=> {

        const fetchBokers = async () => {

            axionsInstance.get("subscriptions").then(response => {
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
            title: 'Node ID',
            dataIndex: 'node_id',
            key: 'node_id',
        },
        {
          title: 'Client Identifier',
          dataIndex: 'clientid',
          key: 'clientid',
        },

        {
            title: 'Client Ip adresss and port',
            dataIndex: 'client_addr',
            key: 'client_addr',
        },

        {
          title: 'QoS',
          dataIndex: 'qos',
          key: 'qos',
        },
        {
          title: 'Share',
          dataIndex: 'share',
          key: 'share',
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

export default Subscriptions