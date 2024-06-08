import { Spin, Table } from "antd";
import React, { useEffect, useState } from "react"
import { axionsInstance } from "../utils/constants";

const Brokers: React.FC = ()=> {
    const [isLoading, setIsLoading] = useState<boolean>(false)
    const [dataSource, setDataSource] = useState<Object[]>([])


    useEffect(()=> {

        const fetchBokers = () => {

            axionsInstance.get("brokers").then(response => {
                console.log("response", response)
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
          title: 'Date Time',
          dataIndex: 'datetime',
          key: 'datetime',
        },
        {
          title: 'Node Id',
          dataIndex: 'node_id',
          key: 'node_id',
        },
        {
          title: 'Node Name',
          dataIndex: 'node_name',
          key: 'node_name',
        },
        {
            title: 'Node Status',
            dataIndex: 'node_status',
            key: 'node_status',
        },

        {
            title: 'Systeme Description',
            dataIndex: 'sysdescr',
            key: 'sysdescr',
        },

        {
            title: 'Up Time',
            dataIndex: 'uptime',
            key: 'uptime',
        },
        {
            title: 'Version',
            dataIndex: 'version',
            key: 'version',
        },
      ];

    return  <>
                {isLoading ? (
                    <Table dataSource={dataSource} columns={columns} />
                    
                    ): (<Spin tip="Loading..." size="large">
                    <div className="content" />
                    </Spin>
                )}
            </>
}

export default Brokers