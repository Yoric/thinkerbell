# When Tinkerers Take Things That Talk Together 

A few notes on scripting in the world of IoT.

## Scenarios: Server Apps

These notes concentrate on code that should executed on a device
behaving as a server for IoT devices.

For the time being, these scenarios are untriaged.

### Temperature setter

> During the day, temperature of the heater should be set to 19C, but
> during the night, reduce the temperature of heaters to 16C.

Input devices:
* current time of day (not an actual sensor/IoT device, but we should
  be able to behave as if it was).

Output device:
* all heaters (not a single IoT device, rather a set of devices).

Additional notes:
* we need to represent time efficiently, e.g. not hammer ourselves
every ms;
* we need to be able to access fields such as
  time-of-day-in-current-timezone.

### Oven safety

> When I leave the house, if the oven is on, send me a message and
> sound a pre-recorded message on the speaker close to the door.

Input devices:
* something that will tell the FoxBox that nobody is home. Perhaps an door-opened detector. Perhaps the owner's cellphone;
* the oven's on/off state.

Output devices:
* message sender (using Firefox Accounts rather than a real IoT
  device, but we should be able to behave as if it was);
* device that can play sound.

Additional note:
* do we want to send an entire sound file to the sound-playing device?

### Light setter

> When the motion detector hasn't seen any movement in 10 minutes,
> turn off the lignts.

Input devices:
* motion detector (note that we are interested in metadata, not current data);
* current time of day (again, not an actual sensor/IoT device).

Output device:
* all lights (not a single IoT device, rather a set of devices).

Additional note:
* this app is not useful all year round, so we need a simple way to
  turn the app on/off from the remote control, without necessarily
  uninstalling it.

### Smart Device detector

> In this Highschool, when a wifi/broadband-enabled device enters or
> is turned on in the exam perimeter, send a message to the desktop
> server. Give as much detail as possible on where the device is, so
> that the teachers can come and frown at offending student.

Input devices:
* wifi detectors;
* broadband detectors.

Output device:
* message sender;

Additional notes:
* we want to be able to send data on *which* sensor informed us;
* we need to be able to send messages.

### Pollution monitor

> If any pollution sensor detects more than n1% of CO2 or n2% of CO,
> etc. send a secure message to a web service.

TBD

### Humidity detector

> If the server room is humid, inform all admins.

TBD

### Supply management

> If there are no more cookies on the shelves of the store, send a
> message to the manager. Don't do this more than once per hour.

TBD

### Art museum painting protectors

> Light sensors detect use of flash. If a flash is detected, ring an
> annoying sound.

TBD

## See also

* [More applications of sensors ](http://www.libelium.com/top_50_iot_sensor_applications_ranking/)
* [More applications](https://temboo.com/iot-applications)
* [And on Wikipedia](https://en.wikipedia.org/wiki/Internet_of_Things#Applications)
