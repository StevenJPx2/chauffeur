import json
from datetime import date
from decimal import Decimal
from pathlib import Path

from .models import DiscountCode, Order, OrderLine, Product


class NotFound(KeyError):
    pass


def normalize_code(code):
    return code.strip().upper()


class Store:
    """All shop state, persisted as a single JSON document."""

    def __init__(self, path):
        self.path = Path(path)
        if self.path.exists():
            self._data = json.loads(self.path.read_text(encoding="utf-8"))
        else:
            self._data = {"products": {}, "orders": []}
        self._data.setdefault("codes", {})

    def save(self):
        self.path.write_text(json.dumps(self._data, indent=2, sort_keys=True), encoding="utf-8")

    def add_product(self, product):
        self._data["products"][product.sku] = {"name": product.name, "price": str(product.price)}
        self.save()

    def get_product(self, sku):
        raw = self._data["products"].get(sku)
        if raw is None:
            raise NotFound(f"unknown product {sku!r}")
        return Product(sku, raw["name"], Decimal(raw["price"]))

    def add_code(self, discount):
        key = normalize_code(discount.code)
        used = self._data["codes"].get(key, {}).get("used_by", [])
        self._data["codes"][key] = {
            "kind": discount.kind,
            "value": str(discount.value),
            "expires": discount.expires.isoformat(),
            "used_by": used,
        }
        self.save()

    def get_code(self, code):
        key = normalize_code(code)
        raw = self._data["codes"].get(key)
        if raw is None:
            raise NotFound(f"unknown discount code {code!r}")
        return DiscountCode(key, raw["kind"], Decimal(raw["value"]), date.fromisoformat(raw["expires"]))

    def code_used_by(self, code, customer):
        raw = self._data["codes"].get(normalize_code(code))
        return raw is not None and customer in raw["used_by"]

    def next_order_id(self):
        return len(self._data["orders"]) + 1

    def record_order(self, order):
        self._data["orders"].append(
            {
                "id": order.id,
                "customer": order.customer,
                "lines": [[line.sku, line.qty] for line in order.lines],
                "subtotal": str(order.subtotal),
                "discount": str(order.discount),
                "code": order.code,
                "total": str(order.total),
                "placed_on": order.placed_on.isoformat(),
            }
        )
        if order.code:
            self._data["codes"][normalize_code(order.code)]["used_by"].append(order.customer)
        self.save()

    def orders_for(self, customer):
        return [
            Order(
                raw["id"],
                raw["customer"],
                [OrderLine(sku, qty) for sku, qty in raw["lines"]],
                Decimal(raw["subtotal"]),
                Decimal(raw["total"]),
                date.fromisoformat(raw["placed_on"]),
                Decimal(raw.get("discount", "0.00")),
                raw.get("code"),
            )
            for raw in self._data["orders"]
            if raw["customer"] == customer
        ]
