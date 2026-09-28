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


@dataclass(frozen=True)
class DiscountCode:
    code: str
    kind: str
    value: Decimal
    expires: date


@dataclass
class Order:
    id: int
    customer: str
    lines: list
    subtotal: Decimal
    total: Decimal
    placed_on: date
    discount: Decimal = Decimal("0.00")
    code: str = None
