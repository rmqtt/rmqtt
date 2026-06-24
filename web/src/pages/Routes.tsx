import { Spin, Table } from "antd";
import React, { useEffect, useState } from "react"
import { axionsInstance } from "../utils/constants";

const Routes: React.FC = ()=> {
    const [isLoading, setIsLoading] = useState<boolean>(false)
    const [dataSource, setDataSource] = useState<Object[]>([])


    useEffect(()=> {

        const fetchBokers = async () => {

            axionsInstance.get("routes").then(response => {
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
            title: 'MQTT Topic',
            dataIndex: 'topic',
            key: 'topic',
        },
        {
          title: 'Node ID',
          dataIndex: 'clientid',
          key: 'clientid',
        }
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

export default Routes