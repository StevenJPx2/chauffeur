from app.core.store import Store
from app.core.render import render


def main():
    store = Store()
    # TODO: load the store path from the command line
    store.load("tasks.json")
    print(render(store.items()))


if __name__ == "__main__":
    main()
