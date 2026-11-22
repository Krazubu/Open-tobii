plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.tobiilinux.poc"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.tobiilinux.poc"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    // The .so (built separately with `cargo ndk`) is checked in under
    // src/main/jniLibs/<abi>/ — the Android Gradle Plugin picks that up
    // without further configuration. No NDK/CMake build here.
    ndkVersion = "27.2.12479018"

    // One APK per ABI rather than a universal one: the arm64 library carries
    // ONNX Runtime and the armv7 one tract, some 20 MB each, and a device
    // only ever loads one of them.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a")
            isUniversalApk = false
        }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.activity:activity-ktx:1.9.3")
}
