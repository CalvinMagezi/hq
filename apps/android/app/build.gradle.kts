plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The hq and Caddy binaries and the built web app are inputs, not repository files. Pass them as
// -Phq.bin=, -Phq.caddy= and -Phq.web= (README). Without them the APK still builds, minus hq, which is
// what CI needs.
val hqBin = providers.gradleProperty("hq.bin")
val caddyBin = providers.gradleProperty("hq.caddy")
val webDir = providers.gradleProperty("hq.web")
val keystore = providers.gradleProperty("hq.keystore")
val generated = layout.buildDirectory.dir("generated/hq")

// A phone build needs all four inputs. A missing or misspelled path would otherwise be skipped and the APK
// would install over the working app without hq, or signed with the wrong key, so fail here instead.
val phoneInputs = mapOf("hq.bin" to hqBin, "hq.caddy" to caddyBin, "hq.web" to webDir, "hq.keystore" to keystore)
if (phoneInputs.values.any { it.isPresent }) {
    phoneInputs.forEach { (name, value) ->
        val path = value.orNull
            ?: throw GradleException("-P$name is missing: a phone build needs hq.bin, hq.caddy, hq.web and hq.keystore together")
        if (!File(path).isAbsolute || !File(path).exists()) {
            throw GradleException("-P$name=$path is not an existing absolute path")
        }
    }
}

val prepareHq = tasks.register<Sync>("prepareHq") {
    into(generated)
    hqBin.orNull?.let { path -> from(path) { into("jniLibs/arm64-v8a"); rename { "libhq.so" } } }
    caddyBin.orNull?.let { path -> from(path) { into("jniLibs/arm64-v8a"); rename { "libcaddy.so" } } }
    webDir.orNull?.let { path -> from(path) { into("assets/web") } }
}
tasks.named("preBuild") { dependsOn(prepareHq) }

android {
    namespace = "example.hq.spike"
    compileSdk = 36

    defaultConfig {
        // The id and the signing key must stay as they are: the installed phone keeps its vault, its
        // Keystore key and its sessions only if an update has the same id and the same signature.
        // -Phq.appId= installs a second copy beside the real app for a test on a phone (its own data, its own
        // Keystore key); it is never a release id. Leave it out for the build that updates the installed app.
        applicationId = providers.gradleProperty("hq.appId").getOrElse("example.hq.spike")
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "hub"
        ndk { abiFilters += "arm64-v8a" }
    }

    signingConfigs {
        create("hq") {
            keystore.orNull?.let { storeFile = file(it) }
            storePassword = "android"
            keyAlias = "debug"
            keyPassword = "android"
        }
    }
    buildTypes {
        debug {
            // Stays debuggable on purpose: `adb shell run-as` is how credentials and a restored vault are placed
            // until the app has its own credential form (see TECHDEBT.md).
            if (keystore.isPresent) signingConfig = signingConfigs.getByName("hq")
        }
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir(generated.map { it.dir("jniLibs") })
            assets.srcDir(generated.map { it.dir("assets") })
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true }

    // Keep every file of the web build; the default pattern drops names that start with "." or "_".
    androidResources { ignoreAssetsPattern = "!.svn:!.git" }

    lint {
        // android.jar 36 holds APIs newer than minSdk 26. This check stops a call to one of them from
        // compiling and then crashing Android 8 to 10 (D8 backports some, such as List.of; lint knows which).
        checkOnly += setOf("NewApi")
        abortOnError = true
    }

    packaging {
        jniLibs {
            // hq and Caddy are executables run from nativeLibraryDir, so they must be unpacked on install
            // and left alone by the strip step.
            useLegacyPackaging = true
            keepDebugSymbols += listOf("**/libhq.so", "**/libcaddy.so")
        }
    }
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.03.01"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.activity:activity-compose:1.12.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.10.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.10.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("com.squareup.okhttp3:okhttp:5.3.2")
    // Scanning a pairing code: CameraX (Apache-2.0, no Google Play services) reads the camera and ZXing
    // (Apache-2.0) decodes the QR code. No ML Kit.
    implementation("androidx.camera:camera-core:1.4.2")
    implementation("androidx.camera:camera-camera2:1.4.2")
    implementation("androidx.camera:camera-lifecycle:1.4.2")
    implementation("androidx.camera:camera-view:1.4.2")
    implementation("com.google.zxing:core:3.5.3")

    testImplementation("junit:junit:4.13.2")
    // The platform's org.json is a stub in JVM unit tests.
    testImplementation("org.json:json:20250517")
}
