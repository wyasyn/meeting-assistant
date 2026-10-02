from fastapi import FastAPI

from app.routers import health

# Bearer-token auth (docs/06-api-contracts.md) is added with the spawn logic in roadmap task 4.1.
app = FastAPI(title="Meeting Assistant sidecar", version="0.1.0")
app.include_router(health.router)
