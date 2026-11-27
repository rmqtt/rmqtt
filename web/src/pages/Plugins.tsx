
import { Spin, Table } from "antd";
import React, { useEffect, useState } from "react"
import { axionsInstance } from "../utils/constants";

const Plugins: React.FC = ()=> {
    const [isLoading, setIsLoading] = useState<boolean>(false)
    const [dataSource, setDataSource] = useState<Object[]>([])


    useEffect(()=> {

        const fetchBokers = async () => {

            axionsInstance.get("plugins").then(response => {
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
            dataIndex: 'node',
            key: 'topic',
        },
        {
          title: 'Plugin name',
          dataIndex: 'plugins.name',
          key: 'plugins.name',
        },
        {
            title: 'Plugin Version',
            dataIndex: 'plugins.version',
            key: 'plugins.version',
        },
        {
            title: 'Plugin description',
            dataIndex: 'plugins.descr',
            key: 'plugins.descr',
        },
        {
            title: 'Plugin description',
            dataIndex: 'plugins.active',
            key: 'plugins.active',
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

export default Plugins