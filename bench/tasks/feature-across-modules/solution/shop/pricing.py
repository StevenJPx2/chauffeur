from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")


class PricingError(ValueError):
    pass


class DiscountError(PricingError):
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


def check_code(store, code, customer, today):
    from .storage import NotFound

    try:
        discount = store.get_code(code)
    except NotFound:
        raise DiscountError(f"unknown discount code {code!r}")
    if today > discount.expires:
        raise DiscountError(f"discount code {code!r} expired on {discount.expires.isoformat()}")
    if store.code_used_by(code, customer):
        raise DiscountError(f"discount code {code!r} was already used by {customer}")
    return discount


def discount_amount(discount, amount):
    if discount.kind == "percent":
        off = money(amount * discount.value / 100)
    elif discount.kind == "fixed":
        off = money(discount.value)
    else:
        raise DiscountError(f"unknown discount kind {discount.kind!r}")
    return min(max(off, Decimal("0.00")), amount)
