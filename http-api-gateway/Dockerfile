FROM python:3.13
WORKDIR /app

COPY . /app

RUN pip install uv
RUN uv sync


EXPOSE 8080

CMD ["uv", "run", "main.py"]