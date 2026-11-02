plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

// The hq and Caddy binaries and the built web app are inputs, not repository files. Pass them as
// -Phq.bin=, -Phq.caddy= and -Phq.web= (README). Without them the APK still builds, minus hq, which is
// what CI needs for its unit tests and lint.
val hqBin = providers.gradleProperty("hq.bin")
val caddyBin = providers.gradleProperty("hq.caddy")
val webDir = providers.gradleProperty("hq.web")
val keystore = providers.gradleProperty("hq.keystore")
val generated = layout.buildDirectory.dir("generated/hq")

// The release key comes from the environment (CI decodes it from a secret); it is never a repository file
// or a Gradle property, so it cannot leak through a build scan or a checked-in gradle.properties.
val releaseKeystore = providers.environmentVariable("HQ_RELEASE_KEYSTORE")
val releaseKeystorePassword = providers.environmentVariable("HQ_RELEASE_KEYSTORE_PASSWORD")
val releaseKeyAlias = providers.environmentVariable("HQ_RELEASE_KEY_ALIAS")
val versionCodeProperty = providers.gradleProperty("hq.versionCode")
val versionNameProperty = providers.gradleProperty("hq.versionName")

fun requireAbsolutePath(name: String, path: String) {
    if (!File(path).isAbsolute || !File(path).exists()) {
        throw GradleException("-P$name=$path is not an existing absolute path")
    }
}

// A phone build needs hq, Caddy and the web app together. A missing or misspelled path would otherwise be
// skipped and the APK would install over the working app without hq, so fail here instead.
val hqInputs = mapOf("hq.bin" to hqBin, "hq.caddy" to caddyBin, "hq.web" to webDir)
if (hqInputs.values.any { it.isPresent } || keystore.isPresent) {
    hqInputs.forEach { (name, value) -> value.orNull?.let { requireAbsolutePath(name, it) } }
    keystore.orNull?.let { requireAbsolutePath("hq.keystore", it) }
}

// Checked when the task graph is known, so a debug build, tests and lint need none of the release inputs.
gradle.taskGraph.whenReady {
    val packaged = allTasks.map { it.path }
    fun missingInputs() = hqInputs.filterValues { !it.isPresent }.keys.map { "-P$it" }
    if (":app:packageRelease" in packaged) {
        val missingKey = mapOf(
            "HQ_RELEASE_KEYSTORE" to releaseKeystore,
            "HQ_RELEASE_KEYSTORE_PASSWORD" to releaseKeystorePassword,
            "HQ_RELEASE_KEY_ALIAS" to releaseKeyAlias,
        ).filterValues { !it.isPresent }.keys
        if (missingKey.isNotEmpty()) {
            throw GradleException(
                "A release build is signed with the release key and with nothing else. Set ${missingKey.joinToString()} " +
                    "(CI does this from its secrets; see apps/android/README.md). Never build a release locally without them."
            )
        }
        if (!File(releaseKeystore.get()).isFile) throw GradleException("HQ_RELEASE_KEYSTORE=${releaseKeystore.get()} is not a file")
        if (missingInputs().isNotEmpty()) {
            throw GradleException("A release build needs hq, Caddy and the web app: ${missingInputs().joinToString()} is missing")
        }
    }
    if (":app:packageDebug" in packaged && hqInputs.values.any { it.isPresent } && !keystore.isPresent) {
        throw GradleException("-Phq.keystore is missing: a phone build needs hq.bin, hq.caddy, hq.web and hq.keystore together")
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
        // The public id and the release key are fixed from here on: an installed phone keeps its vault, its
        // Keystore key and its sessions only if an update has the same id and the same signature. The Java
        // and Kotlin package (namespace) is a different thing and stays example.hq.spike.
        // -Phq.appId= installs a second copy beside the real app for a test on a phone (its own data, its own
        // Keystore key); it is never a release id. Leave it out for the build that updates the installed app.
        applicationId = providers.gradleProperty("hq.appId").getOrElse("online.agenthq.app")
        minSdk = 26
        targetSdk = 35
        // CI passes a number that only grows (the workflow run number); a phone refuses a lower one.
        versionCode = versionCodeProperty.map(String::toInt).getOrElse(1)
        versionName = versionNameProperty.getOrElse("dev")
        // shortcuts.xml cannot use a manifest placeholder, so its target package is a resource.
        resValue("string", "hq_application_id", applicationId!!)
        ndk { abiFilters += "arm64-v8a" }
    }

    signingConfigs {
        create("hq") {
            keystore.orNull?.let { storeFile = file(it) }
            storePassword = "android"
            keyAlias = "debug"
            keyPassword = "android"
        }
        create("release") {
            releaseKeystore.orNull?.let { storeFile = file(it) }
            // One password protects the store and the key.
            storePassword = releaseKeystorePassword.orNull
            keyAlias = releaseKeyAlias.orNull
            keyPassword = releaseKeystorePassword.orNull
            enableV1Signing = true
            enableV2Signing = true
            enableV3Signing = true
        }
    }
    buildTypes {
        debug {
            // Stays debuggable on purpose: `adb shell run-as` is how credentials and a restored vault are placed
            // until the app has its own credential form (see TECHDEBT.md).
            if (keystore.isPresent) signingConfig = signingConfigs.getByName("hq")
        }
        release {
            isDebuggable = false
            // hq and Caddy are shipped as they are, so there is no code to shrink.
            isMinifyEnabled = false
            // Left unset without the key; the task graph check above stops the build first.
            if (releaseKeystore.isPresent) signingConfig = signingConfigs.getByName("release")
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
            // sherpa-onnx's JNI library needs only libonnxruntime; its C and C++ API libraries are for other callers.
            excludes += listOf("**/libsherpa-onnx-c-api.so", "**/libsherpa-onnx-cxx-api.so")
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
    // Hearing and speaking on the phone (Apache-2.0): speech to text and text to speech run in this process, so
    // no audio reaches Google or anyone else. Its models are fetched once with pinned checksums (voice/VoiceModels.kt).
    implementation("com.github.k2-fsa.sherpa-onnx:sherpa-onnx:v2.0.1")
    // Unpacks the models' .tar.bz2 releases (Apache-2.0).
    implementation("org.apache.commons:commons-compress:1.27.1")

    testImplementation("junit:junit:4.13.2")
    // The platform's org.json is a stub in JVM unit tests.
    testImplementation("org.json:json:20250517")
}
