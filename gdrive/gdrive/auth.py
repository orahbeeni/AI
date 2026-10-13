"""OAuth for an installed app. Token is stored 0600 and refreshed automatically."""
import os

from google.auth.transport.requests import Request
from google.oauth2.credentials import Credentials
from google_auth_oauthlib.flow import InstalledAppFlow

from . import config

SCOPES = ["https://www.googleapis.com/auth/drive"]


class AuthError(Exception):
    pass


def login():
    if not config.CLIENT_SECRET.exists():
        raise AuthError(f"Missing {config.CLIENT_SECRET} - see README.md, step 1.")
    flow = InstalledAppFlow.from_client_secrets_file(str(config.CLIENT_SECRET), SCOPES)
    creds = flow.run_local_server(port=0, prompt="consent", access_type="offline")
    _save(creds)
    return creds


def credentials():
    if not config.TOKEN.exists():
        raise AuthError("Not signed in. Run: gdrive login")
    creds = Credentials.from_authorized_user_file(str(config.TOKEN), SCOPES)
    if not creds.valid:
        if creds.refresh_token:
            creds.refresh(Request())
            _save(creds)
        else:
            raise AuthError("Sign-in expired. Run: gdrive login")
    return creds


def logout():
    config.TOKEN.unlink(missing_ok=True)


def _save(creds):
    config.CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    fd = os.open(config.TOKEN, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(creds.to_json())
