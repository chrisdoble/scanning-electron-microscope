import pyvisa
import time
import warnings

resource_manager = pyvisa.ResourceManager('@py')

# list_resources warns about TCPIP resource discovery being limited because
# certain packages aren't installed. We don't care about that so ignore them.
with warnings.catch_warnings():
    warnings.simplefilter("ignore")
    resources = resource_manager.list_resources('USB?*::?*::DHO8?*::INSTR')

assert len(resources) == 1, "expected exactly 1 matching resource"
scope = resource_manager.open_resource(resources[0])

# Stop the scope
scope.write(':STOP')

# Clear the screen
scope.write(':CLEar')

# Show channel 1
scope.write(':CHANnel1:DISPLAY ON')

# Set the offset of channel 1 to 0 V
scope.write(':CHANnel1:OFFSet 0')

# Set the vertical scale of channel 1 to 1 V/div
scope.write(':CHANnel1:SCALe 1')

# Clear all measurement items
scope.write(':MEASure:CLEar')

# Run the scope
scope.write(':RUN')

# Start calculating the average voltage of channel 1
scope.write(':MEASure:ITEM? VAVG,CHANnel1')

# Give the scope some time to sample
time.sleep(0.5)

# Query the average voltage of channel 1
print(scope.query(':MEASure:ITEM? VAVG,CHANnel1'))