plugins {
    id("com.android.application")
}

android {
    namespace = "org.handover.android"
    compileSdk = 37

    defaultConfig {
        applicationId = "org.handover.android"
        minSdk = 26
        targetSdk = 37
        versionCode = 52
        versionName = "0.3.3"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        val path = providers.environmentVariable("HANDOVER_ANDROID_SIGNING_FILE").orNull
        if (!path.isNullOrBlank()) {
            create("handoverRelease") {
                storeFile = file(path)
                storePassword = providers.environmentVariable("HANDOVER_ANDROID_STORE_PASSWORD").get()
                keyAlias = providers.environmentVariable("HANDOVER_ANDROID_KEY_ALIAS").get()
                keyPassword = providers.environmentVariable("HANDOVER_ANDROID_KEY_PASSWORD").get()
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("handoverRelease")
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-stdlib")
    implementation("androidx.core:core:1.9.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
    androidTestImplementation("androidx.test:core:1.5.0")
    androidTestImplementation("androidx.test.ext:junit:1.1.5")
    androidTestImplementation("androidx.test:runner:1.5.2")
}
