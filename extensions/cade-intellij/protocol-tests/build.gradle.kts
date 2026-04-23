// Lightweight subproject for testing protocol serialisation without the
// IntelliJ Platform SDK. No IDE download required.
plugins {
    id("org.jetbrains.kotlin.jvm") version "1.9.25"
    id("org.jetbrains.kotlin.plugin.serialization") version "1.9.25"
}

repositories { mavenCentral() }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.6.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.1")
    testImplementation(kotlin("test"))
    testImplementation(kotlin("test-junit5"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.8.1")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher:1.10.2")
}

kotlin { jvmToolchain(17) }
tasks.test { useJUnitPlatform() }
