package com.example.gst_audio_flutter

import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.media.AudioManager
import android.os.Build
import android.service.notification.StatusBarNotification
import android.view.KeyEvent
import com.ryanheise.audioservice.AudioServiceActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : AudioServiceActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        if (!BuildConfig.DEBUG) {
            return
        }

        MethodChannel(
            flutterEngine.dartExecutor.binaryMessenger,
            "com.example.gst_audio_flutter/native_audio_test",
        ).setMethodCallHandler { call, result ->
            when (call.method) {
                "dispatchMediaKey" -> {
                    val keyCode = call.argument<Int>("keyCode")
                        ?: KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE
                    dispatchMediaKey(keyCode)
                    result.success(null)
                }
                "isMusicActive" -> result.success(audioManager().isMusicActive)
                "activeNotifications" -> result.success(activeNotifications())
                "sdkInt" -> result.success(Build.VERSION.SDK_INT)
                else -> result.notImplemented()
            }
        }
    }

    private fun dispatchMediaKey(keyCode: Int) {
        val now = System.currentTimeMillis()
        val audioManager = audioManager()
        audioManager.dispatchMediaKeyEvent(
            KeyEvent(now, now, KeyEvent.ACTION_DOWN, keyCode, 0),
        )
        audioManager.dispatchMediaKeyEvent(
            KeyEvent(now, now, KeyEvent.ACTION_UP, keyCode, 0),
        )
    }

    private fun audioManager(): AudioManager {
        return getSystemService(Context.AUDIO_SERVICE) as AudioManager
    }

    private fun activeNotifications(): List<Map<String, Any?>> {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
            return emptyList()
        }
        val notificationManager =
            getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        return notificationManager.activeNotifications.map { statusBarNotification ->
            statusBarNotification.toDebugMap()
        }
    }

    private fun StatusBarNotification.toDebugMap(): Map<String, Any?> {
        val extras = notification.extras
        return mapOf(
            "packageName" to packageName,
            "id" to id,
            "tag" to tag,
            "isOngoing" to notification.isOngoing(),
            "category" to notification.category,
            "title" to extras.getCharSequence(Notification.EXTRA_TITLE)?.toString(),
            "text" to extras.getCharSequence(Notification.EXTRA_TEXT)?.toString(),
            "subText" to extras.getCharSequence(Notification.EXTRA_SUB_TEXT)?.toString(),
        )
    }

    private fun Notification.isOngoing(): Boolean {
        return flags and Notification.FLAG_ONGOING_EVENT != 0 ||
            flags and Notification.FLAG_FOREGROUND_SERVICE != 0
    }
}
