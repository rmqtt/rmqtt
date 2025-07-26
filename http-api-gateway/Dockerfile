FROM python:3.13
WORKDIR /app

ADD pyproject.toml /app/pyproject.toml

RUN pip install uv
RUN uv venv
RUN uv pip install .
COPY ./ /app

EXPOSE 8080

CMD ["uv", "run", "main.py"]
