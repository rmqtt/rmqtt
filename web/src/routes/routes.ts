import Clients from '../pages/Clients';
import Metrics from '../pages/Metrics';
import Nodes from '../pages/Nodes';
import Plugins from '../pages/Plugins';
import Publish from '../pages/Publish';
import Routes from '../pages/Routes';
import Stats from '../pages/Stats';
import Subscribes from '../pages/Subscribes';
import Subscriptions from '../pages/Subscriptions';
import Brokers from '../pages/Brokers';


const router =  [

    {
        path: "/brokers",
        component: Brokers
    },

    {
        path: "/clients",
        component: Clients
    },

    {
        path: "/metrics",
        component: Metrics
    },

    {
        path: "/nodes",
        component: Nodes
    },

    {
        path: "/plugins",
        component: Plugins
    },

    {
        path: "/publishs",
        component: Publish
    },

    {
        path: "/routes",
        component: Routes
    },

    {
        path: "/stats",
        component: Stats
    },

    {
        path: "/subscribes",
        component: Subscribes
    },

    {
        path: "/subscriptions",
        component: Subscriptions
    }
]

export default router;