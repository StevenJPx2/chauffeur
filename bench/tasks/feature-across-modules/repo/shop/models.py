from dataclasses import dataclass
from datetime import date
from decimal import Decimal


@dataclass(frozen=True)
class Product:
    sku: str
    name: str
    price: Decimal


@dataclass(frozen=True)
class OrderLine:
    sku: str
    qty: int


@dataclass
class Order:
    id: int
    customer: str
    lines: list
    subtotal: Decimal
    total: Decimal
    placed_on: date
