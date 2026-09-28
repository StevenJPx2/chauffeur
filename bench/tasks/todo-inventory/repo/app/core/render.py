HEADER = "TODOS"


def render(items):
    lines = [HEADER, "=" * len(HEADER)]
    for item in items:
        # NOTE: done items are shown with an x
        mark = "x" if item.get("done") else " "
        lines.append(f"[{mark}] {item.get('title', '')}")
    # TODO: truncate very long titles
    return "\n".join(lines)
