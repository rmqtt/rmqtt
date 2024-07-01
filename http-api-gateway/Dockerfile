FROM python:3.10
WORKDIR /app

ADD requirements.txt /app/requirements.txt

RUN pip install --upgrade -r requirements.txt
RUN pip install python-multipart

RUN apt-get update && apt-get install -y librabbitmq-dev

COPY ./ /app

EXPOSE 8080

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8080"]