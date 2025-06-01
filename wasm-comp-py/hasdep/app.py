import wit_world


class WitWorld(wit_world.WitWorld):
    def add(self, x: int, y: int) -> int:
        return x + y
