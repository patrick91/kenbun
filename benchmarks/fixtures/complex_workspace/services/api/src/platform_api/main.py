from fastapi import FastAPI

from platform_api.routes import router

app = FastAPI(title="Platform API")
app.include_router(router)
