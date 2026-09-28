from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")


class PricingError(ValueError):
    pass


def money(value):
    """Round to whole cents, halves rounding up."""
    return Decimal(value).quantize(CENT, rounding=ROUND_HALF_UP)


def line_total(product, qty):
    if qty <= 0:
        raise PricingError(f"quantity for {product.sku} must be positive, got {qty}")
    return money(product.price * qty)


def subtotal(store, lines):
    return money(sum((line_total(store.get_product(line.sku), line.qty) for line in lines), Decimal(0)))
