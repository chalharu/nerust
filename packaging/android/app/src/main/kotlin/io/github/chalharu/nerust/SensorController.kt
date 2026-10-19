package io.github.chalharu.nerust

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.view.InputDevice
import android.view.Surface as AndroidSurface

internal fun screenRelativeAcceleration(
    x: Float,
    y: Float,
    rotation: Int,
): FloatArray {
    val (screenX, screenY) =
        when (rotation) {
            AndroidSurface.ROTATION_0 -> x to y
            AndroidSurface.ROTATION_90 -> -y to x
            AndroidSurface.ROTATION_180 -> -x to -y
            AndroidSurface.ROTATION_270 -> y to -x
            else -> x to y
        }
    return floatArrayOf(
        screenX / SensorManager.GRAVITY_EARTH,
        -screenY / SensorManager.GRAVITY_EARTH,
    )
}

/**
 * Cartridge motion sensor + rumble routing.
 *
 * Extracted from MainActivity. Owns registration (resumed + focused +
 * motion-enabled) and vibrator selection (handset vs controller target);
 * the activity forwards lifecycle calls and peripheral configuration.
 */
class SensorController(
    private val activity: MainActivity,
) : SensorEventListener {
    private lateinit var sensorManager: SensorManager
    private var accelerometer: Sensor? = null
    var activityResumed = false
    var windowHasFocus = false
    var motionEnabled = false
    private var sensorRegistered = false
    var rumbleEnabled = false
    var rumbleStrengthPercent = 100
    var rumbleTarget = "auto"
    var rumbleIntensity = 0
    var lastControllerDeviceId: Int? = null
    private var activeRumbleVibrator: Vibrator? = null
    private var activeRumbleAmplitude = 0

    fun onCreate() {
        sensorManager = activity.getSystemService(Context.SENSOR_SERVICE) as SensorManager
        accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
    }

    fun onResume() {
        activityResumed = true
        updateMotionRegistration()
        applyRumble()
    }

    fun onWindowFocusChanged(hasFocus: Boolean) {
        windowHasFocus = hasFocus
        if (hasFocus) {
            updateMotionRegistration()
            applyRumble()
        } else {
            updateMotionRegistration()
            cancelRumble()
        }
    }

    fun onPause() {
        activityResumed = false
        updateMotionRegistration()
        cancelRumble()
    }

    fun onDestroy() {
        onPause()
        sensorRegistered = false
        accelerometer = null
    }

    fun configure(
        motionEnabled: Boolean,
        rumbleEnabled: Boolean,
        rumbleStrengthPercent: Int,
        rumbleTarget: String,
    ) {
        this.motionEnabled = motionEnabled
        this.rumbleEnabled = rumbleEnabled
        this.rumbleStrengthPercent = rumbleStrengthPercent.coerceIn(0, 100)
        this.rumbleTarget =
            when (rumbleTarget) {
                "handset", "controller" -> rumbleTarget
                else -> "auto"
            }
        updateMotionRegistration()
        applyRumble()
    }

    fun setIntensity(intensity: Int) {
        rumbleIntensity = intensity.coerceIn(0, 255)
        applyRumble()
    }

    fun noteControllerInput() {
        applyRumble()
    }

    fun noteControllerRemoved(deviceId: Int) {
        if (lastControllerDeviceId == deviceId) {
            cancelRumble()
            lastControllerDeviceId = null
            applyRumble()
        }
    }

    private fun updateMotionRegistration() {
        if (!::sensorManager.isInitialized) return
        val shouldRegister =
            activityResumed && windowHasFocus && motionEnabled && accelerometer != null
        if (shouldRegister == sensorRegistered) return
        if (shouldRegister) {
            sensorRegistered =
                sensorManager.registerListener(
                    this,
                    accelerometer,
                    SensorManager.SENSOR_DELAY_GAME,
                )
            if (!sensorRegistered) activity.forwardCartridgeAcceleration(0f, 0f)
        } else {
            sensorManager.unregisterListener(this)
            sensorRegistered = false
            activity.forwardCartridgeAcceleration(0f, 0f)
        }
    }

    @Suppress("DEPRECATION")
    override fun onSensorChanged(event: SensorEvent) {
        if (event.sensor.type != Sensor.TYPE_ACCELEROMETER || !sensorRegistered) return
        val sample =
            screenRelativeAcceleration(
                event.values[0],
                event.values[1],
                activity.windowManager.defaultDisplay.rotation,
            )
        activity.forwardCartridgeAcceleration(sample[0], sample[1])
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) = Unit

    private fun applyRumble() {
        val scaledIntensity =
            rumbleIntensity * rumbleStrengthPercent / 100
        val vibrator =
            if (activityResumed && windowHasFocus && rumbleEnabled && scaledIntensity > 0) {
                selectVibrator()
            } else {
                null
            }
        if (vibrator == null || !vibrator.hasVibrator()) {
            cancelRumble()
            return
        }
        val amplitude =
            if (vibrator.hasAmplitudeControl()) {
                scaledIntensity.coerceIn(1, 255)
            } else {
                VibrationEffect.DEFAULT_AMPLITUDE
            }
        if (activeRumbleVibrator === vibrator && activeRumbleAmplitude == amplitude) return
        cancelRumble()
        vibrator.vibrate(
            VibrationEffect.createWaveform(
                longArrayOf(1_000L),
                intArrayOf(amplitude),
                0,
            ),
        )
        activeRumbleVibrator = vibrator
        activeRumbleAmplitude = amplitude
    }

    private fun cancelRumble() {
        activeRumbleVibrator?.cancel()
        activeRumbleVibrator = null
        activeRumbleAmplitude = 0
    }

    private fun selectVibrator(): Vibrator? =
        when (rumbleTarget) {
            "handset" -> handsetVibrator()
            "controller" -> controllerVibrator()
            else -> controllerVibrator() ?: handsetVibrator()
        }

    @Suppress("DEPRECATION")
    private fun handsetVibrator(): Vibrator =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            activity.getSystemService(VibratorManager::class.java).defaultVibrator
        } else {
            activity.getSystemService(Context.VIBRATOR_SERVICE) as Vibrator
        }

    @Suppress("DEPRECATION")
    private fun controllerVibrator(): Vibrator? {
        val device = lastControllerDeviceId?.let(InputDevice::getDevice) ?: return null
        val vibrator =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                device.vibratorManager.defaultVibrator
            } else {
                device.vibrator
            }
        return vibrator.takeIf(Vibrator::hasVibrator)
    }
}
