app = FastAPI()

# Target backend server to which you want to forward requests
BACKEND_URL = "http://rmqtt:6060/api/v1"

@app.api_route("/{path:path}", methods=["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD"])
async def proxy(request: Request, path: str):
    async with httpx.AsyncClient() as client:
        # Construct full URL to target
        url = f"{BACKEND_URL}/{path}"
        
        # Forward method, headers, and body
        forwarded_request = client.build_request(
            method=request.method,
            url=url,
            headers=request.headers.raw,
            content=await request.body()
        )

        # Send the request
        backend_response = await client.send(forwarded_request, stream=True)

        # Return the response to the client
        return Response(
            content=await backend_response.aread(),
            status_code=backend_response.status_code,
            headers=dict(backend_response.headers)
        )