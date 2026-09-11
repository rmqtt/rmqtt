import { Link } from 'react-router-dom';

interface BreadcrumbProps {
  pageName: string;
  title: string;
}


const Breadcrumb = ({ pageName, title }: BreadcrumbProps) => {
  return (
    <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
      <h2 className="text-title-md2 font-semibold text-black dark:text-white">
        {pageName}
      </h2>

      <nav>
        <ol className="flex items-center gap-2">
          <li>
            <Link to="/">Accueil /</Link>
          </li>
          <li className="text-primary">{title}</li>
        </ol>
      </nav>
    </div>
  );
};

export default Breadcrumb;