import json
from datetime import date
from decimal import Decimal
from pathlib import Path

from .models import Order, OrderLine, Product


class NotFound(KeyError):
    pass


class Store:
    """All shop state, persisted as a single JSON document."""

    def __init__(self, path):
        self.path = Path(path)
        if self.path.exists():
            self._data = json.loads(self.path.read_text(encoding="utf-8"))
        else:
            self._data = {"products": {}, "orders": []}

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

    def next_order_id(self):
        return len(self._data["orders"]) + 1

    def record_order(self, order):
        self._data["orders"].append(
            {
                "id": order.id,
                "customer": order.customer,
                "lines": [[line.sku, line.qty] for line in order.lines],
                "subtotal": str(order.subtotal),
                "total": str(order.total),
                "placed_on": order.placed_on.isoformat(),
            }
        )
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
            )
            for raw in self._data["orders"]
            if raw["customer"] == customer
        ]
