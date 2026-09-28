import json


class Store:
    def __init__(self):
        self.todo_items = []

    def load(self, path):
        try:
            with open(path, encoding="utf-8") as fh:
                self.todo_items = json.load(fh)
        except FileNotFoundError:
            self.todo_items = []

    def save(self, path):
        # TODO: write to a temp file and rename for atomic saves
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(self.todo_items, fh)

    def items(self):
        return list(self.todo_items)
