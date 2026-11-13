import { BrowserRouter as Router, Routes, Route } from 'react-router-dom';
import LayoutApp from './Layout';
import Home from './pages/Home';
import router from './routes/routes';


function App() {
  return (
    <Router>
      <Routes>
        <Route path="/" element={<LayoutApp/>}>
          <Route index element={<Home />} />
          {
            router.map(route => <Route path={route.path} Component={route.component} />)
          }
        </Route>
      </Routes>
    </Router>
  );
}

export default App;