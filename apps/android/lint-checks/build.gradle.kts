// The app's own lint rules (docs/plans/ux-run/resolutions.md D-11): words a person reads come from
// the string catalogue, never from a literal in a composable.
plugins {
    id("org.jetbrains.kotlin.jvm")
}

// Lint's version is the Android Gradle plugin's plus 23.
val lintVersion = "31.11.1"

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    compileOnly("com.android.tools.lint:lint-api:$lintVersion")
    testImplementation("com.android.tools.lint:lint-api:$lintVersion")
    testImplementation("com.android.tools.lint:lint-tests:$lintVersion")
    testImplementation("junit:junit:4.13.2")
}

tasks.jar {
    manifest { attributes("Lint-Registry-v2" to "example.hq.lint.HqIssueRegistry") }
}
