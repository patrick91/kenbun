from fastapi import FastAPI

from fixture_api.routes import router

app = FastAPI(title="Fixture API")
app.include_router(router)
