import pyvisa
import warnings

resource_manager = pyvisa.ResourceManager('@py')

# list_resources warns about TCPIP resource discovery being limited because
# certain packages aren't installed. We don't care about that so ignore them.
with warnings.catch_warnings():
    warnings.simplefilter("ignore")
    resources = resource_manager.list_resources('USB?*::?*::DHO8?*::INSTR')

assert len(resources) == 1, "expected exactly 1 matching resource"
scope = resource_manager.open_resource(resources[0])
print(scope.query("*IDN?"))