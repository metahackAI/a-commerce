import pytest

from a_commerce.seed import fresh_store


@pytest.fixture
def store():
    return fresh_store()


JAMIE = {"shopper": True, "member_id": "mem_jamie", "tier": "vip"}
