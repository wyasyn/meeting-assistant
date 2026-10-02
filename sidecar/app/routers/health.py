from fastapi import APIRouter
from pydantic import BaseModel


class HealthResponse(BaseModel):
    status: str
    models_loaded: list[str]


router = APIRouter()


@router.get("/health")
def health() -> HealthResponse:
    # No models are loaded until roadmap task 4.1 adds the services.
    return HealthResponse(status="ok", models_loaded=[])
