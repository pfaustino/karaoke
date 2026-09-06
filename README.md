# Karaoke Booth

Live voice booth: amplify your mic, autotune, reverb, and character effects (alien, chipmunk, demon, robot, telephone, chorus, radio). Native Windows exe uses WASAPI for lower latency than the browser demo.

## Native app (Windows)

```powershell
npm run exe
native\target\release\karaoke-booth.exe
```

Headphones on. For Discord, check **Discord mode**, enable Windows **Stereo Mix** (Sound → Recording → Show Disabled Devices), and set Discord’s input to Stereo Mix. Keep the Windows **default recording device** on your microphone. Turn off Discord noise suppression.

## Browser demo

```powershell
npm install
npm run dev
```

Open the Vite URL, allow the microphone, and use headphones. Latency is higher than the exe because of the browser audio path.

```powershell
npm run check
```
