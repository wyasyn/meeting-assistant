from fastapi.testclient import TestClient

from app.main import app


def test_health_reports_ok_with_no_models_loaded() -> None:
    client = TestClient(app)

    response = client.get("/health")

    assert response.status_code == 200
    assert response.json() == {"status": "ok", "models_loaded": []}
