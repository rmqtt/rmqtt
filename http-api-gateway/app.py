from fastapi import Request, FastAPI, APIRouter
from fastapi.middleware.cors import CORSMiddleware
import requests


app = FastAPI(description="HTTP API Gateway that supports cross-domain functionality.", contact={"email": "samanidarix@gmail.com", "tel": "691439424"},title="RMQTT API GATEWAY")


prefix_router = APIRouter()


origins = ["*"]

app.add_middleware(
    CORSMiddleware,
    allow_origins=origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.get("/", tags=["Root"])
async def read_root():
    return {"message": "Welcome to the RMQTT HTTP API GATEWAY."}

@prefix_router.get("/brokers")
async def get_brokers():
    
    response = requests.get('http://rmqtt:6060/api/v1/brokers')
    return response.json()



@prefix_router.get("/nodes")
def get_nodes():

    response = requests.get("'http://rmqtt:6060/api/v1/nodes")
    return response.json()


@prefix_router.get("/clients")
async def get_clients():
    
    response = requests.get('http://rmqtt:6060/api/v1/clients')
    return response.json()



app.include_router(prefix_router, prefix="/api/v1", tags=["BROKERS"])
    